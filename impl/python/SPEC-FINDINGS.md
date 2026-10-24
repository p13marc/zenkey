# Findings against the zk2 spec (from the Python implementation, #609)

zk2py was written from `spec/` alone: `core.md`, the two JSON schemas,
`core/`, the fixtures and the scenarios, plus `examples/zk2/` as extra
inputs. It never read the Rust implementation or `docs/zk2/`, and it runs
the Rust owner example only as a black box. Each entry below is a place
where that was not enough, or where the spec said two things.

**Fifty-five rounds.**
- F-01 to F-39 were found against `core.md` 0.2.
- F-40 to F-45 were found against 0.4.
- F-46 to F-55 come from the live half's first slice.
- F-56 to F-63 were found against 0.5.
- F-64 to F-70 were found against 0.6, with the rest of the live half.
- F-71 to F-73 were found against 0.7.
- F-74 to F-76 were found against 0.8.
- Nothing new was found against 0.9 (see "At 0.9").
- F-77 to F-79 were found against 0.10.
- F-80 was found against 0.11.
- F-81 was found against 0.12.
- Nothing new was found against 0.13 (see "At 0.13").
- F-82 to F-88 were found against 0.14, building access control from §11.
- Nothing new was found against 0.15 (see "At 0.15").
- F-89 to F-91 were found against 0.16, whose rules came from the
  reference's tools rather than from zk2py.
- F-92 and F-93 were found against 0.17.
- Amendments 0.5 to 0.18 resolved F-01 to F-93. Each entry carries a
  status line naming its amendment.
- Nothing new was found against 0.18 (see "At 0.18").
- F-94 to F-97 were found against `hostid.v1`'s text 0.1, the first
  profile, read cold with core 0.19 (see "At 0.19: hostid.v1 0.1").
  From here, an entry's location says which text it is against.
- `hostid.v1` 0.2 resolved F-94 to F-97.
- F-98 was found against core 0.20, and F-99 against `hostid.v1` 0.2 (see
  "At core 0.20 and hostid.v1 0.2"). Core 0.22 resolved F-98, and
  `hostid.v1` 0.3 resolved F-99.
- F-100 to F-102 were found against `freshness.v1`'s text 0.1, the second
  profile, read cold with core 0.21 and 0.22 (see "At 0.21–0.22:
  freshness.v1 0.1"). `freshness.v1` 0.2 resolved them.
- F-103 and F-104 were found against `health.v1`'s text 0.1, the third
  profile, read cold with core 0.23 and `freshness.v1` 0.2 (see "At 0.23:
  health.v1 0.1"). `health.v1` 0.2 resolved them.
- Nothing new was found against `health.v1` 0.2, read and run live against
  the Rust owner example (see "At health.v1 0.2").
- F-105 was found against core 0.24's §2.7, the population budget, read cold
  with `health.v1` 0.3 and `hostid.v1` 0.4 (see "At 0.24: the population
  budget"). Core 0.25 resolved it.
- Nothing new was found against core 0.25 (see "At 0.25").
- F-106 was found against core 0.26's §4.2, a late joiner's seed (see "At
  0.26: a late joiner's seed"). Core 0.27 resolved it.
- Nothing new was found against core 0.27 (see "At 0.27").
- F-107 to F-113 were found against `config.v1`'s text 0.1, the fourth
  profile, read cold with core 0.27 and `freshness.v1` 0.3 (see "At
  config.v1 0.1"). `freshness.v1` 0.3 changed only an appendix, and nothing
  was found in it. `config.v1` 0.2 resolved F-107 to F-113.
- F-114 and F-115 were found against `config.v1` 0.2 and core 0.28, read
  cold and run live against the Rust owner example's `config.v1` (see "At
  config.v1 0.2 and core 0.28"). `config.v1` 0.3 resolved them.
- F-116 and F-117 were found against `link.v1`'s text 0.1, the fifth
  profile, read cold with core 0.29 and run on zenoh-python routers (see
  "At link.v1 0.1, config.v1 0.3 and core 0.29"). Nothing was found in
  `config.v1` 0.3 or core 0.29 themselves. `link.v1` 0.2 and core 0.30
  resolved F-116 and F-117.
- F-118 and F-119 were found against `link.v1` 0.2, reading four questions
  the reference had to answer by reading (see "At link.v1 0.2 and core
  0.30"). Nothing was found in core 0.30. `link.v1` 0.3 resolved them.
- F-120 was found against `link.v1` 0.3, reading four more questions and
  running its scenarios §7 and §8 (see "At link.v1 0.3"). `link.v1` 0.4
  resolved it.
- F-121 was found against `link.v1` 0.4, reading three questions and
  running its scenarios §9 and §10 (see "At link.v1 0.4"). `link.v1` 0.5
  resolved it.
- F-122 was found against `link.v1` 0.5, running its scenario §11 (see
  "At link.v1 0.5"). Core 0.31 and `link.v1` 0.6 resolved it.
- F-123 was found against core 0.31's §11.2, reading its new bullet (see
  "At core 0.31 and link.v1 0.6"). Core 0.32 resolved it.
- F-124 and F-125 were found against `link.v1` 0.6, in a last cold read of
  §2.4, §2.15 and §5 (see "At core 0.32"). `link.v1` 0.7 resolved them.
- Nothing new was found against `link.v1` 0.7, read cold once more (see
  "At link.v1 0.7").
- F-126 to F-132 were found against `alarms.v1`'s text 0.1, the sixth
  profile, read cold with core 0.33 and 0.34 (see "At alarms.v1 0.1").
  Nothing was found in core 0.33 or 0.34 themselves. `alarms.v1` 0.2
  resolved all seven, F-127 against zk2py's reading.
- F-133 and F-134 were found against `alarms.v1` 0.2, read cold and run
  live against the Rust owner example's `--alarms` (see "At alarms.v1
  0.2"). F-133 is against the example's documentation, not the spec.
  `alarms.v1` 0.3 resolved both.
- F-135 to F-137 were found against `alarms.v1` 0.3, read cold and run
  live against the owner example at 0.3 (see "At alarms.v1 0.3"). F-137
  came from review of PR #809. `alarms.v1` 0.5 resolved all three, F-137
  partly against zk2py's reading.
- F-138 and F-139 were found against `alarms.v1` 0.4 and 0.5, read cold
  and run live against the owner example at 0.3 (see "At alarms.v1 0.5").
  `alarms.v1` 0.6 resolved both, as zk2py read them.
- F-140 was found against `alarms.v1` 0.6, read cold and run live (see
  "At alarms.v1 0.6"). `alarms.v1` 0.7 resolved it.
- F-141 was found against `alarms.v1` 0.7, in a final whole-text pass
  (see "At alarms.v1 0.7"). `alarms.v1` 0.8 resolved it.
- Nothing new was found against `alarms.v1` 0.8, read cold, run live and
  passed over whole once more: none open (see "At alarms.v1 0.8").
- F-142 to F-149 were found against `telemetry.v1`'s text 0.1, the seventh
  profile, read cold with core 0.35 and run live against zk2py's owner (see
  "At telemetry.v1 0.1"). Nothing was found in core 0.35 or `link.v1` 0.8
  themselves. `telemetry.v1` 0.2 resolved all eight, F-143 partly and
  F-144 wholly against zk2py's reading.
- F-150 and F-151 were found against `telemetry.v1` 0.2, read cold and run
  live against zk2py's owner and the Rust owner example's `--telemetry`
  (see "At telemetry.v1 0.2"). `telemetry.v1` 0.3 resolved both, as zk2py
  read them.
- F-152 to F-155 were found against `telemetry.v1` 0.3, read cold and run
  with the Rust tool, `zenctl telemetry`, judging zk2py's owner beside
  zk2py's reader (see "At telemetry.v1 0.3" at the end). F-152 is against
  the text; F-153 to F-155 are against the tool's help and output.
  `telemetry.v1` 0.4 resolved F-152 as zk2py read it, and the tool
  answered F-153 to F-155 (`zenctl-te`).
- F-156 to F-158 were found against `telemetry.v1` 0.4, read cold, judged
  again with the fixed tool, and read through once more as a whole (see
  "At telemetry.v1 0.4"). `telemetry.v1` 0.5 resolved all three, F-157
  against zk2py's reading.
- F-159 was found against `zenctl telemetry` (`zenctl-tf`), judging
  `telemetry.v1` 0.5. Nothing was found in the text, which is closed (see
  "At telemetry.v1 0.5"). The tool resolved F-159 in TG.
- F-160 was found against `zenctl-tg`'s "listening" line (see "At
  telemetry.v1 0.5, the tool after TG"). The tool resolved it in TH.
- Nothing new was found against `zenctl-th` (see "At telemetry.v1 0.5,
  the tool after TH"). None is open, and telemetry.v1's barrier
  work, rounds 45 to 51, is closed.
- F-161 to F-180 were found against `desired.v1`'s text 0.1, the eighth
  profile, read cold with core 0.36 in round 52, its scenarios read for
  sufficiency only (see "At desired.v1 0.1"). F-161 to F-174
  are against the text, its schema and its CHANGELOG, F-175 to F-180
  against `scenarios.md`. Nothing was found in core 0.36 itself.
  `desired.v1` 0.2 and `config.v1` 0.4 resolved all twenty, F-163, F-166
  (partly), F-168 and F-170 against zk2py's reading.
- F-181 to F-184 were found against `desired.v1` 0.2 and `config.v1` 0.4,
  read cold in round 53 (see "At desired.v1 0.2"). All four are open, and
  minor; `desired.v1` 0.3 answers them in a later round.
- F-185 to F-191 were found against core 0.37, what a complete presence
  read is, read cold in round 54, with `scenarios/presence.md` §7 to §9
  run from Python (see "At core 0.37"). `desired.v1` 0.3 answers F-187's
  and F-191's `desired.v1` halves; the rest are open.
- `desired.v1` 0.3 resolved F-181 (against zk2py's reading), F-183 and
  F-184, and `config.v1` 0.5 F-182.
- F-192 to F-198 were found against `desired.v1` 0.3 and `config.v1` 0.5,
  read cold and, for the first time, run live in round 55. The parties were
  zk2py's own author and target, the Rust owner example as a black box,
  and one of each (see "At desired.v1 0.3" at the end). F-194 is against
  the owner example's help. All seven are open.

**Severities.**
- **gap:** the prose is silent. The entry says whether a fixture's expected
  value decided it, or zk2py guessed.
- **ambiguity:** the text allows two readings. A fixture or a guess decided.
- **contradiction:** the prose, read literally, and a fixture disagree, or
  two parts of the spec do.
- **blocker:** zk2py could not implement the rule. None was found.

**Counts at core 0.32, hostid.v1 0.5, freshness.v1 0.3, health.v1 0.4,
config.v1 0.3 and link.v1 0.7:** 125 entries, none open.
- F-01 to F-55: resolved by 0.5.
- F-56 to F-63: resolved by 0.6.
- F-64 to F-70: resolved by 0.7.
  - 0.7 confirmed six of zk2py's guesses: F-64, F-65, F-66, F-67, F-68 and
    F-69.
  - It overturned one, F-70 (exposed). F-67's place for the put was refined
    too.
  - The Rust owner example has met F-65 and F-68 since FH2 (#660), and the
    runner now checks both strictly. With its new `--connect`, it also runs
    in F-69's setup, behind a router.
- F-71 to F-73: resolved by 0.8.
  - 0.8 overturned two of zk2py's guesses, F-71 (`["null"]`) and F-72 (by
    text).
  - It resolved F-73 by withdrawing the scenario's claim to observe the
    tick from outside.
- F-74 to F-76: resolved by 0.9.
  - 0.9 confirmed zk2py's two guesses, F-74 (O2 first) and F-76 (zero
    values, then completion).
  - It made zk2py's fix of F-75 the scenario's step.
  - It also fixed the owner example's silent templated operations, which
    the runner had reported as two known deviations. Both are plain checks
    now.
- F-77 to F-79: resolved by 0.11.
  - 0.11 confirmed all three of zk2py's guesses: `optional` unchecked
    against the contract and `false` as absent (F-77), zids by value
    (F-78), and the `plugins` reading as agreeing with the named storages
    selector for a router with no plugin (F-79).
  - F-78 was a bug in the reference's doctor, which compared zid text.
- F-80: resolved by 0.12, against zk2py's fix. Judging an answer by the
  zid in its key was not enough: a spoofer answers on the real router's
  own key. 0.12 judges by the reply's replier id.
- F-81: resolved by 0.13. Routers are verified outward through the
  session lists of routers already verified, so a client tool verifies a
  far router through its own router.
- F-82 to F-88: resolved by 0.15. §11 now states the input, the message
  and flow table, the Tool's admin read, the complement's key set, and
  "every principal" per policy. It fixed what a storage is (F-86) and
  security.md §3 step 3 (F-88). Each resolution follows zk2py's
  measurement or guess.
- F-89 to F-91: resolved by 0.17. 0.17 confirmed two of zk2py's
  guesses: F-90's grants and F-91's refusal at step 2, which 0.17 also
  backs with a D code. It overturned one part of F-89: a foreign stamp is
  a finding even when no router is verified.
- F-92 and F-93: resolved by 0.18, in wording only.
  - F-92 went against zk2py's placement of D011, after cascade 3. The new
    fixture `d011-bad-fingerprint` failed until D011 moved before it.
  - F-93 confirmed zk2py's reading.
- F-94 to F-97: resolved by `hostid.v1` 0.2, written against core 0.20.
  - 0.2 confirmed one of zk2py's guesses, F-95: the first service that asks
    fixes the setting, and a failure mints nothing.
  - It overturned two. F-96: a racer's file absent after `EEXIST` is
    `absent` and fails closed, ephemeral or not, where zk2py had called it
    "not created". F-97: §2.12 counts only instances whose minting §5's
    first question establishes, where zk2py had counted by the listing.
  - F-94 changed zk2py's root from a plain seam to a chroot-like resolution
    of paths. zk2py had kept links out of its seam's reach.
- F-98: resolved by core 0.22, against zk2py's guess. Two majors of one
  profile in one contract's `uses` are now E002. zk2py's linter reports it,
  and zk2py splits `zk2py_order.v1` in two.
- F-99: resolved by `hostid.v1` 0.3. The ephemeral start is logged wherever
  the process's logs go, at its warning level. The owner example writes its
  runtime's logs to stderr, and py-live's check passes, no longer XFAIL.
- F-100 to F-102: resolved by `freshness.v1` 0.2.
  - F-100 in wording, as zk2py read it. zk2py's subscriber and GET reader
    are unchanged.
  - It overturned two of zk2py's guesses. F-101: with no observation at
    all, the horizon's steps still answer, so no horizon is not asked.
    F-102: over one reading, the offset closest to zero decides, and a
    stamp ahead beyond the delta withdraws the trust. zk2py had trusted on
    any one put for the reader's life.
- F-103 and F-104: resolved by `health.v1` 0.2.
  - F-103 kept zk2py's measurement and its check. Without root, the fault's
    stamp is not checked. 0.2 adds a [moved clock] tier, which zk2py now
    runs: an owner without an HLC, stamping from its offset clock.
  - F-104 overturned zk2py's guess. The tool takes the deployment's word for
    its clock, where zk2py had subscribed and measured.
- F-105: resolved by core 0.25, against zk2py's guess. "Lost no delivery"
  is established from what the reader can see, never assumed: the reader
  lost none, and the resource is `reliable` with `block`. zk2py had read
  it as knowledge, so a best-effort window was lossless unless it knew of
  a loss. Now no such window is.
- F-106: resolved by core 0.27. Its unstamped rule is zk2py's guess, and
  so is R6 for a tool's seed. It overturned zk2py's order on equal times:
  ids compare as zenoh compares them, their 16 bytes little-endian from
  the first, where zk2py had compared them as integers.
- F-107 to F-113: resolved by `config.v1` 0.2.
  - F-107 overturned zk2py's guess for the values: a value outside
    `ParamValue` is refused before row 1. Its order among members, `values`
    last, is zk2py's guess, now stated.
  - F-108 and F-113 state zk2py's guesses as rules. F-109 and F-110 state
    what the fixtures had decided, as zk2py read them. F-112 states zk2py's
    reader.
  - F-111 kept zk2py's guess for a parameter an applied group of the
    pending change holds, and decided a branch zk2py had not: a parameter
    only a reach group not yet applied set reads `overlay` at once. zk2py
    had left it at `runtime`, and failed two steps of the new
    `lifecycle.json` case 25 before it followed.
- F-114 and F-115: resolved by `config.v1` 0.3, both as zk2py guessed
  and as the owner example behaved. F-115's range is the unsigned 64-bit
  one, where zk2py had used the signed; no case falls between.
- F-116 and F-117: resolved by `link.v1` 0.2 and core 0.30, both as zk2py
  measured and guessed: the scenarios' readers subscribe to their grants,
  and the declaration grant covers the owner's queryables as declared.
- F-118 and F-119: resolved by `link.v1` 0.3. F-119 as zk2py guessed,
  failing closed over every revision given, with the least horizon and a
  warning added. F-118 kept zk2py's guess, the denies in the far
  principal's policy, and added what it lacked: every other principal's
  subject narrowed off the face, and under `allow` the face's own subject.
- F-120: resolved by `link.v1` 0.4. Its three blocks are reported as
  zk2py measured them, with two more of the same kind, and its reading for
  each is the rule, with `face_subject_misplaced` in place of a check of
  the far principal's policy.
- F-121: resolved by `link.v1` 0.5, as zk2py measured it and read it.
  0.5 also found the same hole in the far principal's subject.
- F-122: resolved by core 0.31 and `link.v1` 0.6. Both complements now
  conform, and zk2py's, the stricter, is named with its exact outcome.
- F-123: resolved by core 0.32, in zk2py's wording.
- F-124 and F-125: resolved by `link.v1` 0.7, each by one of the readings
  zk2py's entry offered: `block_disabled`, and the union with
  `far_of_two_faces`.

Code comments cite open entries as `SPEC-FINDINGS F-nn`, and resolved ones
by the spec section that now states the rule.

| Id | Severity | Status at core 0.32, config.v1 0.3, link.v1 0.7 | Location | In one line |
|---|---|---|---|---|
| F-01 | ambiguity | resolved by 0.5 | §1.2 ULID | No first-character bound. |
| F-02 | ambiguity | resolved by 0.5 | §1.1 | Is `x-eth0` a valid resource chunk without a contract? |
| F-03 | ambiguity | resolved by 0.5 | §2.2 | Do `uint` parameters only match canonical decimal? |
| F-04 | gap | resolved by 0.5 | §3.3 | D000–D004, D008, D010 undefined in prose. |
| F-05 | gap | resolved by 0.5 | descriptors/ | Cascades and scope only in fixtures. |
| F-06 | ambiguity | resolved by 0.5 | §3.3, R3 | Implied descriptor checks no fixture pinned. |
| F-07 | contradiction | resolved by 0.5 | §3.3 example | Holds `imu`, yet "no IMU". |
| F-08 | ambiguity | resolved by 0.5 | §5.2 | Envelope encodings and type errors. |
| F-09 | ambiguity | resolved by 0.5 | §7.3, E037 | Refused keywords' content. |
| F-10 | ambiguity | resolved by 0.5 | §9.4, E032 | `$ref` scheme and fragments. |
| F-11 | gap | resolved by 0.5 | §9.4, §9.6 | Cross-file `$ref` in a bundle. |
| F-12 | gap | resolved by 0.5 | §9.4, E029 | Duplicate member in a schema file. |
| F-13 | ambiguity | resolved by 0.5 | E024 | Counting duplicate stems. |
| F-14 | gap | resolved by 0.5 | §9.4 | Well-known type sources. |
| F-15 | ambiguity | resolved by 0.5 | §9.4 | Names, roots, nested messages, enums, `json_name`. |
| F-16 | ambiguity | resolved by 0.5 | §9.1 | TOML's 64-bit integer limit. |
| F-17 | ambiguity | resolved by 0.5 | contract.schema.json | Bounds only in `format`. |
| F-18 | ambiguity | resolved by 0.5 | §9.1, E020 | Which TOML date/time kinds count. |
| F-19 | gap | resolved by 0.5 | §9.1, §9.5 | Floats: `nan`/`inf`, `1e16`, `1.0` ≡ `1`. |
| F-20 | ambiguity | resolved by 0.5 | E020 | Counting; requirement annotations. |
| F-21 | gap | resolved by 0.5 | §10, W105 | A profile without a vocabulary. |
| F-22 | ambiguity | resolved by 0.5 | E018, E033 | Written or resolved values. |
| F-23 | ambiguity | resolved by 0.5 | §9.2 | Small lint edges. |
| F-24 | gap | resolved by 0.5 | §9.6 | The base64 variant. |
| F-25 | gap | resolved by 0.5 | §9.6 | Entry shapes. |
| F-26 | gap | resolved by 0.5 | §9.6 extras | Source of extras; value shape; empty `extras`. |
| F-27 | ambiguity | resolved by 0.5 | §9.7 | History check order. |
| F-28 | ambiguity | resolved by 0.5 | §9.7 | Retention identity. |
| F-29 | ambiguity | resolved by 0.5 | §9.8 | "Both directions". |
| F-30 | gap | resolved by 0.5 | §9.8 | Pairing resources (see F-40). |
| F-31 | gap | resolved by 0.5 | §9.8 | The contract table's coverage. |
| F-32 | gap | resolved by 0.5 | §9.8 | The JSON Schema rules' coverage. |
| F-33 | ambiguity | resolved by 0.5 | §9.8 | `oneOf`/`anyOf` branch identity. |
| F-34 | contradiction | resolved by 0.5 | §9.8 | A renumber "is a deletion" yet no warning. |
| F-35 | gap | resolved by 0.5 | §9.8 | The protobuf rules' coverage. |
| F-36 | gap | resolved by 0.5 | compat/ | The compat input layout. |
| F-37 | gap | resolved by 0.5 | fixture documents | Meanings deferred to Rust symbols. |
| F-38 | gap | resolved by 0.5 | §9.2, sets/ | A failing member; E035 against duplicates. |
| F-39 | ambiguity | resolved by 0.5 | E021/E022 | "The first" without key order. |
| F-40 | contradiction | resolved by 0.5 | §9.8 | "Matched by kind token" against `explicit_cleared`. |
| F-41 | ambiguity | resolved by 0.5 | §9.1 | "MAY accept later TOML" against E000. |
| F-42 | contradiction | resolved by 0.5 | CHANGELOG, compat/ | Stale counts and descriptions. |
| F-43 | ambiguity | resolved by 0.5 | §9.8 | Explicit presence. |
| F-44 | ambiguity | resolved by 0.5 | compat/README.md | Wrapper location; `invalid`. |
| F-45 | gap | resolved by 0.5 | §9.7 | Where a contract's history lives. |
| F-46 | gap | resolved by 0.5 | §3.3 | The descriptor GET's parameters. |
| F-47 | gap (measured) | resolved by 0.5 | §8.1 | The handler rule binds only the GET. |
| F-48 | gap | resolved by 0.5 | §8.4 | The bundle reply's encoding. |
| F-49 | gap | resolved by 0.5 | §8.1, §3.3, §8.4 | No timeouts. |
| F-50 | gap (measured) | resolved by 0.5 | §8.4 | Consolidation `None`. |
| F-51 | ambiguity | resolved by 0.5 | §8.4 | "Nearest holder" for a client. |
| F-52 | gap | resolved by 0.5 | §3.2 | An unbound required role. |
| F-53 | ambiguity | resolved by 0.5 | §3.3 | `profiles`. |
| F-54 | gap | resolved by 0.5 | §8.1 | When a member exists. |
| F-55 | ambiguity | resolved by 0.5 | §3.2 R3 | An unbound optional role. |
| F-56 | ambiguity | resolved by 0.6 | §2.6, E026 (0.5) | "The seconds MUST fit 64 bits": signed or unsigned? |
| F-57 | ambiguity | resolved by 0.6 | §3.3 D008, D010 (0.5) | "Once for the repeat": once per repeated value, or once per descriptor? |
| F-58 | ambiguity | resolved by 0.6 | §5.2 CBOR (0.5) | "An integer outside 64 bits": i64, u64, or their union? |
| F-59 | ambiguity | resolved by 0.6 | §8.1 timeouts (0.5) | "Waits for presence after an owner starts … 1 s": from which instant? |
| F-60 | ambiguity | resolved by 0.6 | §9.8 `oneof_branch_added` (0.5) | "More branches than the earlier one's" when one side has no `oneOf`. |
| F-61 | gap | resolved by 0.6 | §3.2, presence.md §2 step 4 (0.5) | "No instance token appears" cannot be observed when the refusing owner is its own router. |
| F-62 | contradiction | resolved by 0.6 | §9.6 against §9.4 (0.5) | One id for identical files, under the *last* name, breaks a bundle `$ref` to the first file's stem. |
| F-63 | ambiguity | resolved by 0.6 | descriptor.schema.json, §3.3, §9.1 (0.5) | `uint64` is bounded for contracts but not for descriptors' `cardinality`. |
| F-64 | ambiguity | resolved by 0.7 | §5.1 O1–O3 | A `replies = "one"` call names no consolidation. |
| F-65 | gap (observed) | resolved by 0.7 | §5.2 | An `app` envelope for an operation with no `error` type, or a raw one, in JSON: what `detail` is. |
| F-66 | ambiguity | resolved by 0.7 | §4.3 minting | "Plus one tick": which unit? |
| F-67 | ambiguity | resolved by 0.7 | §3.3, §8.2 | Is the first descriptor a "change" to put, and where in the bring-up order? |
| F-68 | gap | resolved by 0.7 | §8.2, §4.2 | When an owner puts its first state value, relative to its tokens. |
| F-69 | gap | resolved by 0.7 | §4.2 S1, state.md §1 | "A router stamp would carry the router's": not observable when the owner is the router. |
| F-70 | gap | resolved by 0.7 | §8.2 step 2 | What "exposed" means for a state resource, or a templated one, at start-up. |
| F-71 | ambiguity | resolved by 0.8 | §7.3 the nullable form (0.7) | "The null schema, whose type is exactly null": is `{"type": ["null"]}` one? |
| F-72 | ambiguity | resolved by 0.8 | §9.8 inside undecided keywords (0.7) | "A $ref back to a target already being followed is compared as written": its text, or its target? |
| F-73 | gap | resolved by 0.8 | state.md §1 step 3 (0.7) | "Faster than its clock advances" cannot be arranged by a tester; the tick path went unexercised. |
| F-74 | ambiguity | resolved by 0.9 | §5.1 "Over a template" against O2 (0.8) | A wildcard call to a fan-out-forbidden template, with a non-canonical parameter chunk: `fanout_forbidden` or `invalid_request`? |
| F-75 | gap | resolved by 0.9 | presence.md §6 step 3 (0.8) | `zk2/**` selects no control token, so its stalled read's "no token" cannot fail. |
| F-76 | ambiguity | resolved by 0.9 | §5.1 "Over a template" and "Answering" (0.8) | A template-wide `replies = "many"` handler that names no member and sends nothing: `internal`, or zero values then completion? |
| F-77 | gap | resolved by 0.11 | §3.3 `requires[].optional` (0.10) | A contract role's `optional` that does not repeat its contract, or an explicit `false`: neither checked nor listed as unchecked. |
| F-78 | gap | resolved by 0.11 | §3.3 `meta.zid`, §4.2 "Observing S1" (0.10) | A zid has no spelling; zenoh writes one without leading zeros, so a textual comparison can call an owner's stamp foreign. |
| F-79 | gap | resolved by 0.11 | §4.2 S4's tool check (0.10) | "The routers' storage admin space": which keys, and what shows a router runs no storage? |
| F-80 | gap | resolved by 0.12 | §4.2 S4, "What the check reads" (0.11) | Any session can answer `@/*/router`: with the routers' admin space off, one record turns "unobservable" into "clean". |
| F-81 | gap | resolved by 0.13 | §4.2 "Who answered" (0.12), Appendix B | Only a router the tool's session is connected to is verified, and a client connects to one: with two routers, a client tool's S4 is never clean. |
| F-82 | gap | resolved by 0.15 | §11, security.md common setup (0.14) | Grants are "generated from contracts and bindings", but no deployment input is specified: who owns, binds, calls, inspects, and which username binds whom. |
| F-83 | gap (measured) | resolved by 0.15 | §11.1 (0.14) | The shapes name actions, not zenoh's messages and flows; a liveliness GET's tokens need egress `liveliness_token`, not `reply`. |
| F-84 | gap (measured) | resolved by 0.15 | §11.1 Tool, §4.2 S4 (0.14) | No shape grants reading the admin space: under `deny` a tool cannot run S4's check. |
| F-85 | ambiguity | resolved by 0.15 | §11.2 "denies of its complement" (0.14) | Key expressions have no negation: over which keys is the complement taken? |
| F-86 | gap (measured) | resolved by 0.15 | §4.2 "What the check reads" (0.11) | The storages selector intersects a router's `router/queryable/<…/state/**>` records, so with any owner present, "nothing under the second" never holds. |
| F-87 | gap (measured) | resolved by 0.15 | §11.1 "denies it to every principal under allow" (0.12) | A subject matching every session undoes the per-user denies in zenoh 1.10.1; "every principal" must be compiled per policy. |
| F-88 | contradiction | resolved by 0.15 | security.md §3 step 3, §11.3 (0.14) | Under `allow`, the grants cannot refuse a session that is no principal: it matches no subject, and gets everything. |
| F-89 | ambiguity | resolved by 0.17 | §4.2 "A tool's S1 check" (0.16) | What S1 reads for a tool that verifies no router: clean by `meta.zid` alone, or unobservable? |
| F-90 | gap | resolved by 0.17 | §5.1 O3 judged from outside (0.16), §11.3 | "Only under grants that let it call": a tool cannot observe its grants, and the spec does not say where it learns them. |
| F-91 | gap | resolved by 0.17 | §4.4 "Found by its token" (0.16) | An archive "MUST hold its archive.v1 interface token", but no step refuses a tokenless one and no check reports it. |
| F-92 | contradiction | resolved by 0.18 | §3.3 D011 against "Cascades and scope" (0.17) | Cascade 6 still lists `token` as deliberately unchecked, and no cascade places D011. |
| F-93 | ambiguity | resolved by 0.18 | §4.2 "A tool's S1 check" (0.17) against "Who answered" (0.13) | "Verified" is two things: a connected router is verified (0.13), yet "verified at least one router" must mean an answer. |
| F-94 | gap | resolved by hostid 0.2 | hostid.v1 0.1, scenarios.md "A root"; §2.4 | A root as a seam: an absolute symbolic link in it resolves outside the root, which no step says how to treat. |
| F-95 | ambiguity | resolved by hostid 0.2 | hostid.v1 0.1 §2.3, §2.7 | When the first service's mint fails closed, is the setting fixed, and does a later service read the inputs again? |
| F-96 | gap | resolved by hostid 0.2 | hostid.v1 0.1 §2.5 step 4, §2.6 | `EEXIST`, then the winner's file absent: is that "not created", which the ephemeral rung replaces? |
| F-97 | ambiguity | resolved by hostid 0.2 | hostid.v1 0.1 §2.12 against §5 | §2.12 counts every instance that lists `hostid.v1`; §5 holds such a listing unobservable when a contract uses it. |
| F-98 | gap | resolved by 0.22 | core 0.20 §3.3 and §9.5, with E020 and §10 point 2 | Two majors of one profile in one contract's `uses` are sorted, but an annotation key names the profile without its major. |
| F-99 | gap | resolved by hostid 0.3 | hostid.v1 0.2 §2.6, scenarios.md §4 expected 1 | "The runtime logs" the ephemeral system, but not where, so a runner of a binary has nowhere to look. |
| F-100 | contradiction | resolved by freshness 0.2 | freshness.v1 0.1 §2.3 against §2.5 and judgements.json | "No value, no verdict … whatever the horizon: unobservable", yet a subscriber that heard nothing is stale past ttl, and fresh at ttl 0. |
| F-101 | ambiguity | resolved by freshness 0.2 | freshness.v1 0.1 §2.7 and judgements.json's description, against §2.3 and §5 | A member of a resource with no horizon that nobody observed: unobservable (`no_observation`), or not asked? |
| F-102 | gap | resolved by freshness 0.2 | freshness.v1 0.1 §2.6, ground 2 | One live put within the delta trusts a clock, but for how long, and does a later put outside it withdraw the trust? |
| F-103 | gap (measured) | resolved by health 0.2 | health.v1 0.1 scenarios.md §4 expected 2, with core 0.23 §4.1 and Appendix B | The fault's stamp "is not the owner's (R1 re-stamped…)" needs the owner's session HLC ahead, which a simulated offset cannot move. |
| F-104 | gap | resolved by health 0.2 | health.v1 0.1 scenarios.md §5–§7, with freshness.v1 0.2 §2.6 | The tool T judges by GET, and nothing says how it trusts its clock: without that, every status is `clock_untrusted`, never healthy. |
| F-105 | ambiguity | resolved by 0.25 | core 0.24 §2.7, "What a tool reads" and its table; budget/population.json, rate.json | A complete window and a rate's clean pole need a window that "lost no delivery": a fact a plain subscription cannot establish, only fail to see contradicted. |
| F-106 | gap | resolved by 0.27 | core 0.26 §4.2 "The merge", with "A selector over several owners" | The merge orders by "the owner's stamp"; a foreign key in a tool's seed has no owner, and its samples may carry no stamp at all. |
| F-107 | ambiguity | resolved by config 0.2 | config.v1 0.1 §2.5, with core §5.1 "The request" and `SetRequest` | A `null`, a fraction, or an integer beyond 64 bits is outside `SetRequest`'s type, refused before row 1 by a decode that holds the schema, and listed under row 11 by the text: where it is refused depends on the decode. |
| F-108 | ambiguity | resolved by config 0.2 | config.v1 0.1 §2.5 "An empty values", against §2.3 and §2.11 | A change of nothing "applies nothing to the device", yet its revision is "as any other's", where the revision counts "the acts that moved the running values". |
| F-109 | gap | resolved by config 0.2 | config.v1 0.1 §2.3 and §2.11, lifecycle.json case 4 | A parameter the owner cannot read back after a failed undo has no value; the text gives it no `source`, and only the fixture says it has none. |
| F-110 | gap | resolved by config 0.2 | config.v1 0.1 §2.17, lifecycle.json case 19 | "600 change events … within the last hour": whether an event exactly an hour old counts is the fixture's, not the text's. |
| F-111 | gap | resolved by config 0.2 | config.v1 0.1 §2.13 "After it", with validation.json | `persist` is accepted while another change is pending; if that change set the same parameter, "its startup value is now its running value" is false, and its undo restores a source the persist made stale. |
| F-112 | gap | resolved by config 0.2 | config.v1 0.1 §2.3 "Two keys, one revision" | A reader pairs the view and the status "by instance and revision", but the documents change without the revision: arming, joining, extending, confirming, persisting. |
| F-113 | gap | resolved by config 0.2 | config.v1 0.1 §2.9, §2.5 row 13 | A reach `set` that starts a change "MUST state" an `apply_at` before the deadline; when its device cannot, no row refuses it, as row 13 refuses a join. |
| F-114 | gap | resolved by config 0.3 | config.v1 0.2 §2.18 with §2.12 | A restart rolls back the values a bound document converged, at the same revision: whether the new instance converges it again, and what its `desired` reads meanwhile, is unsaid. |
| F-115 | ambiguity | resolved by config 0.3 | config.v1 0.2 §2.5 "A request outside its type", with `SetRequest` and `ControlRequest` | `confirm_s` or `expected_revision` written `30.0`, or beyond 64 bits, is of the request's type by 0.2's own reading of `values`, yet both implementations refuse it at the decode, and no row would refuse it otherwise. |
| F-116 | gap | resolved by link 0.2 | link.v1 0.1 scenarios.md "Conventions" and §5, with core §11.1, §11.2 | G and L subscribe to `zk2/lab/*/**` under N's `deny` block compiled from per-binding grants; zenoh 1.10.1 refuses a declaration or a GET no grant includes, so as written they receive nothing. |
| F-117 | gap | resolved by link 0.2, core 0.30 | link.v1 0.1 §2.5, restating core §8.5, with core S2 | The far principal's `declare_queryable` egress grant "for what its side may query" does not include the owner's state queryable, declared over `state/**`: a state GET across a south region gets no reply, a call does. |
| F-118 | ambiguity | resolved by link 0.3 | link.v1 0.2 §2.4 | The denies go into "each subject that can match a session on the face", yet a near principal's subject, matched by its user name alone, can: measured, `local`'s credentials over the face see the tokens and every push. |
| F-119 | gap | resolved by link 0.3 | link.v1 0.2 §2.2, §2.3, §2.7, with core §9.8 | Two revisions of one interface, which may differ in `link.exposure` (an annotation change is a review), can run in one deployment; the plan's denies are per interface, and the text names no revision. |
| F-120 | gap | resolved by link 0.4 | link.v1 0.3 §2.15, with §2.4 | The two subject checks pass three blocks §2.4 forbids, measured on a router: a subject bound to the far principal and a near one, a face subject that also names a near link protocol, and under `allow` the far principal's subject narrowed to its face. |
| F-121 | gap | resolved by link 0.5 | link.v1 0.4 §2.15 `narrowed_too_far`, with §2.4 | `narrowed_too_far` credits a name to a subject that names it beside a name of the other kind, or as the other kind, which zenoh matches only for a session presenting both: measured, `local` narrowed off `tcp` matches no subject over TCP, its puts reach the near side, and the tool reports nothing. |
| F-122 | ambiguity | resolved by core 0.31, link 0.6 | link.v1 0.5 scenarios.md §11 expected 7, with core §11.2 | Under `allow`, X, which matches only a face subject listed by `ground`'s policy and `radio-a`'s, receives nothing on zk2py's router, where the text expects what §1's G receives: zk2py's `radio-a` policy denies what `radio-a` holds no grant for, and core §11.2 does not say whether a generator's complement does. |
| F-123 | ambiguity | resolved by core 0.32 | core 0.31 §11.2 "A principal's own keys" | The new MUST names keys, "the keys of the services it does not run that its grants do not name", not keys per message and flow: read per key, a key a Consume grant names would need no deny of a `put` on it. `security.md` §2 decides it per message and flow. |
| F-124 | gap | resolved by link 0.7 | link.v1 0.6 §2.15, §5 | A block read from a router's configuration whose access control is not enabled (`enabled` false, or absent, zenoh's default) enforces nothing, and passes every check: measured, G and X both get `local`'s share and their puts reach the near side, and the tool reports nothing. |
| F-125 | gap | resolved by link 0.7 | link.v1 0.6 §2.4, §2.1 | Two faces of one policy with one far principal: §2.4 puts both plans' denies into that principal's policy, which holds on both faces, so each face carries the other's denies; nothing lints, refuses or splits it. Measured: G over a face that exposes `link` receives none of its pushes beside a face that exposes none. |

---

## §1 Identity and grammar

### F-01 · ambiguity · §1.2, ULID chunks

**Status at 0.5: resolved by 0.5.** §1.2 now says the check is lexical and the first character has no bound (`8zzz…` is a ULID chunk); zk2py's guess stands.

> "A ULID chunk (events, §2.6) is 26 characters of Crockford's base32 in
> lowercase: digits, and lowercase letters except `i`, `l`, `o`, `u`."

A real ULID is 128 bits, so its first character is `0`–`7`. The spec does
not say whether `8zzzzzzzzzzzzzzzzzzzzzzzzz` is accepted. `keys.json`
has no such case.
**Resolved:** a guess. zk2py follows the literal text and accepts any of
the 26 characters (`lexical.ULID`).

### F-02 · ambiguity · §1.1 (position 6+), `keys.json`

**Status at 0.5: resolved by 0.5.** §1.1 now says a key is parsed lexically, so `x-eth0` is a resource chunk; zk2py's guess stands.

> "6+ | resource chunks | Built from the resource's template (§2.2)."

`keys.json` parses keys without any contract, so a parser cannot know the
template. Is `zk2/s/svc/nav.v2/stream/x-eth0` a zk2 key?
- `x-eth0` is a plain chunk.
- It cannot be a literal (§2.2: a literal "does not start with `x-`").
- It is not a canonical slug (§1.4; `slugs.json` refuses it).

So no template could ever produce it.
**Resolved:** a guess. A resource chunk only has to be a plain chunk
(`keys._data`).

### F-03 · ambiguity · §2.2, `uint` parameters in resolution

**Status at 0.5: resolved by 0.5.** §2.2 now says parameter types play no part in matching, and a tie leaves the first listed template; zk2py's guess stands.

> "A `uint` value is written in decimal without leading zeros, then slugged
> like any value."

> "A template matches a key's resource chunks when … every parameter chunk
> decodes (§1.4)…"

Resolution mentions only decoding. Does `{n}` typed `uint` match the chunk
`03` or `abc`? `templates.json` gives templates without parameter types.
**Resolved:** a guess. Types are ignored when matching (`templates.match`).

## §3.3 The descriptor

### F-04 · gap · §3.3 and `conformance/descriptors/`: most D codes have no definition

**Status at 0.5: resolved by 0.5.** §3.3 now has "The checks": a D000–D010 table with severity (D006 alone a warning) and counting.

> "a checker MUST report exactly the `D…` codes that
> `conformance/descriptors/expect.json` lists for each document, checked
> against the fixture contract."

The prose names four codes:
- D005: "A listed resource that is not an optional resource of the contract is an error";
- D006: "One that a missing capability already implies is a warning";
- D007: `[F: descriptors/d007-*]` after the cardinality rule;
- D009: `[F: descriptors/d009-*]` after the `requires` rule.

D000, D001, D002, D003, D004, D008 and D010 are defined nowhere. There is
no table of D codes like §9.2's table of E and W codes. Nothing says which
D codes are warnings besides D006. `descriptors/expect.json` describes
itself as "the sorted D… codes `check()` reports": `check()` is a Rust
function.

**Resolved:** from the fixtures. Each meaning was reverse-engineered from
the file names (`d002-instance`, `d008-twice`, `d010-profile`, …) and the
documents' single differences. The derived table is in the docstring of
`zk2py/descriptor.py`.

This is where the urge to read the Rust checker was strongest.

### F-05 · gap · `descriptors/`: cascades and scope stated only by fixtures

**Status at 0.5: resolved by 0.5.** §3.3 "Cascades and scope" states all five cascades; zk2py now also drops an invalid `iface` from `declared_by` (cascade 2).

Several rules about how the descriptor checks interact exist only as
expected values:
- **`d003-fingerprint`** expects `["D003"]` for `"contract": "sha256:FEA2"`. A
  malformed fingerprint is D003 *alone*: the revision check (D004) must not
  also run.
- **`d003-iface`** expects `["D003", "D009"]` for `"iface": "nav.2"`. The
  D009 comes from `declared_by: "nav.v2"` no longer naming an interface the
  descriptor lists. The prose says only "A role declared in a contract
  names that contract's interface in `declared_by`".
- **"A document whose interface names another contract is checked for
  syntax only"** appears only in `descriptors/expect.json`'s description.
  - With it, `ok-unknown-revision` (interface `geo.v1`) is clean.
  - `d004-revision` (`nav.v2` with another fingerprint) is D004.

  The case names suggest the reverse. "Unknown revision" is in fact an
  unknown *interface*, and D004 is the unknown revision.

**Resolved:** from the fixtures.

### F-06 · ambiguity · §3.3 and R3: checks the prose implies but no fixture pins

**Status at 0.5: resolved by 0.5.** §3.3 lists what is deliberately not checked; a lowered bound of 0 is D007, a profile twice is D010. Counting a repeat leaves F-57.

These are left unimplemented, except the two duplicate rules at the end,
which are guesses:
- **R3:** "the descriptor MUST list every requirement". Is a role that the
  checked contract declares in `[requires]`, but that is missing from the
  descriptor's `requires`, a D009?
- **`declared_by: "nav.v2"`:** must the role exist in that contract's
  `[requires]` with the same `interface`?
- **`cause`:** must it agree with the resource's gate kind? For example,
  `cause: "capability"` on a resource gated only by `config:`.
- **A lowered bound of `0`:** is it allowed? The schema says `minimum: 0`;
  §2.2 says a contract's cardinality is "a positive integer".
- **`params` values:** are they checked against the required interface's
  templates?
- **`minor`:** the schema has it as `uint64`, while the contract's is
  `uint32`.
- **Duplicates (guesses):** zk2py reports a capability listed twice as
  D008, which the fixture `d008-twice` pins, and a profile listed twice as
  D010, which nothing pins.

### F-07 · contradiction (informative text) · §3.3, the descriptor example

**Status at 0.5: resolved by 0.5.** The example now holds `imu` and lists covariance with cause `config`.

The example has `"capabilities": ["imu", "gnss"]`. It also has
`"unavailable": [{"resource": "state/covariance", "cause": "capability",
"reason": "no IMU"}]`.

The instance holds `imu`, yet it gives "no IMU" as the capability reason
for covariance's absence. If it lacked the IMU, `imu` would not be listed,
and the entry would then be D006 ("a missing capability already implies"
it). The example is informative, but a reader checking it against the
rules finds it inconsistent.
**Resolved:** nothing to implement. Reported here.

## §5.2 The error envelope

### F-08 · ambiguity · §5.2, encodings and type errors

**Status at 0.5: resolved by 0.5.** §5.2 lists the decoding edges. zk2py's guess was overturned: a CBOR byte string in a detail is base64 text, not `bytes_hex`. "An integer outside 64 bits" leaves F-58.

> "The reply's `Encoding` MUST say which: `application/json` or
> `application/cbor`; or `application/protobuf` with the schema suffix
> `zk2.core.v1.Error`."

> "It MUST refuse: an unknown encoding (`encoding`); malformed bytes, a
> duplicate or unknown member, or a missing `code` or `message`
> (`decode`); …"

The spec leaves these open:
- **Variant encodings:** `application/json;charset=utf-8`, plain
  `application/protobuf`, or `application/protobuf;other.Error`. zk2py
  refuses them all with `encoding`, because §7.2 forbids schema suffixes
  except on the envelope.
- **Wrong JSON/CBOR member types:** for `"code": 3` or `"cause": 3`, the
  tag could be `decode`, `code` or `cause`. zk2py uses `decode`, as a shape
  error.
- **A known protobuf field with the wrong wire type:** zk2py uses `decode`.
- **CBOR features:** tags (zk2py decodes the content), `undefined` (read as
  null), indefinite lengths (accepted).
- **CBOR byte strings inside a `detail`:** how to show them. zk2py uses
  `{"bytes_hex": …}`, as the fixtures show a protobuf detail.
- **A protobuf `cause` present but empty:** zk2py refuses it with `cause`.

**Resolved:** guesses, as listed.

## §7.3 and §9.4 JSON Schema artifacts

### F-09 · ambiguity · §7.3, E037 inside refused keywords

**Status at 0.5: resolved by 0.5.** §7.3 defines schema positions once. zk2py's guess was overturned: a refused keyword's content is not walked, and `definitions` is refused without being walked.

> "Refused: every other keyword in a schema position (`pattern`,
> `patternProperties`, `allOf`, `not`, `if`/`then`/`else`, …)"

In JSON Schema 2020-12, the values of `allOf`, `not`, `if`,
`patternProperties` and the other applicators are themselves schema
positions. So in `{"allOf": [{"pattern": "x"}]}`, does `pattern` count as
an E037 too? `e037-subset` cannot tell, because its nested keywords are all
allowed ones.

A related case: `definitions` (draft-07) is a refused keyword, but is its
content walked?
**Resolved:** a guess. zk2py walks every 2020-12 applicator position,
refused ones included, and reports each keyword once per file. It does not
walk `definitions`.

### F-10 · ambiguity · §9.4, E032 and "a scheme"

**Status at 0.5: resolved by 0.5.** §9.4: a scheme is a `:` in the file part; the fragment is a JSON Pointer used as written. zk2py's guess was overturned: no percent-decoding.

> "A `$ref` with a scheme (`:`) is refused. Its pointer MUST resolve."

Open points:
- Is any colon refused, so that `#/$defs/a:b` fails? Or only a URI scheme
  before the fragment?
- Are non-pointer fragments (`#anchor`) refused? zk2py refuses them; the
  subset has no `$anchor` keyword anyway.
- Is the pointer percent-decoded first? The fragment of a URI normally is.

**Resolved:** a guess. A colon in the file part (before `#`) is a scheme.
Pointer fragments only, percent-decoded (`schemas.SchemaSet.resolve_ref`).

### F-11 · gap · §9.4 and §9.6, cross-file `$ref` once bundled

**Status at 0.5: resolved by 0.5.** §9.4: in a bundle, a file part names the artifact whose `name` is the stem of its last path segment. But see F-62: §9.6's duplicate-id rule breaks the premise.

**Status at 0.4: open.** 0.3 adds "`$ref`s are followed, across the revision's artifacts" to §9.8, which confirms that a bundle's `$ref`s must be followed, but still not how a path maps to an artifact that has only a stem.

> "A `$ref`'s file part resolves as a path relative to the referencing
> file, lexically normalized, and MUST name a listed file."

A bundle carries each JSON Schema artifact with only its `name` (the stem).
It keeps no path. Two parts of the spec need those `$ref`s resolved from a
bundle:
- §7.2: "generic tools MUST decode";
- §9.8: the classifier reads earlier revisions from the history.

Neither can resolve `../common/types.json#/$defs/X` by path. Only
`examples/zk2/README.md`, which is not normative, says "Cross-file `$ref`
in a bundle resolves by the referenced file's stem".
**Resolved:** a guess. zk2py takes the stem of the file part's last path
component (`compat.JsonWorld`). Stems are unique per contract (E024).

### F-12 · gap · §9.4 and E029, duplicate members in a JSON Schema file

**Status at 0.5: resolved by 0.5.** §9.4/E029: a duplicate member makes the file not JSON; zk2py's guess stands.

> E029: "a schema file missing, unreadable, not JSON, or not compiling"

The artifact id is "the document's JCS bytes", which a document with
duplicate members does not have. The spec does not say whether such a
document is "not JSON". (§9.6 says it for bundles: "JSON with no duplicate
member".)
**Resolved:** a guess. A duplicate member is E029.

### F-13 · ambiguity · §9.2 E024, counting

**Status at 0.5: resolved by 0.5.** E024 falls once per later file with a taken stem, and that file is not loaded, so `json:stem#Name` looks in the first. zk2py now skips loading it.

> "E024 | a `json:` name defined by several listed files, or two listed
> JSON Schema files with one stem | per reference or file"

For two files sharing a stem, is that one E024 or two? And how does a
qualified reference `json:stem#Name` resolve when the stem is ambiguous?
**Resolved:** a guess. One E024 per file whose stem an earlier file
already took. A qualified reference looks only in the first file with that
stem.

## §9.4 Protobuf artifacts

### F-14 · gap · §9.4, the well-known types' source files

**Status at 0.5: resolved by 0.5.** §9.4 names protoc 3.21.12's `include/` sources for the well-known types.

> "The well-known types are always available, each compiled on demand as
> its own artifact named `google/protobuf/<file>.proto`"

The artifact is the compiled bytes, so its id depends on the *text* of
`empty.proto` and the other files. That text has changed across protobuf
releases (`go_package`, `csharp_namespace`, `objc_class_prefix`, …). The
spec pins the compiler, protoc 3.21.12 in §9.5, but not the source of these
files.

`ok-protobuf-well-known` passes with the files protoc 3.21.12 ships, in its
release archive's `include/` and in Debian's `/usr/include/google/protobuf`.
The two agree byte for byte.

A related point: a user file that imports `google/protobuf/timestamp.proto`
relies on the compiler's built-in include path. The spec does not say so.
**Resolved:** from the fixture. zk2py uses the compiler's own include
directory, with protoc pinned (`protoc.compile_well_known`).

### F-15 · ambiguity · §9.4, names, roots, nested messages, enums, `json_name`

**Status at 0.5: resolved by 0.5.** §9.4: the first import root names a file, E029 under none; a nested message is a message, an enum is E023; listed files shadow the well-known types; every field carries `json_name`.

> "Import roots are `proto_include`, relative to the contract. … Its
> `name` is the file's path relative to its import root."

Open points:
- **Listed paths and roots.** Listed paths are relative to the contract's
  directory. A file under no import root has no name; a file under several
  roots has several. zk2py takes the first root, and reports E029 when
  there is none.
- **Nested messages.** "A message reference resolves to the one listed
  file that defines it." Is `pkg.Outer.Inner` such a message? zk2py says
  yes.
- **Enums.** A reference to an enum is E023 in zk2py.
- **Shadowing.** A listed file could define `google.protobuf.Empty` itself.
  zk2py resolves to the listed file first.
- **`json_name`.** The spec does not say that the artifact includes a
  `json_name` for *every* field, as `protoc --descriptor_set_out` writes it.
  It matters for byte portability, and for §9.7's "every default
  `json_name` dropped".

**Resolved:** the fixtures decide `json_name` (protoc's output matches
every id). The rest are guesses.

## §9.1 TOML and the authoring shape

### F-16 · ambiguity · §9.1, the TOML integer range

**Status at 0.5: resolved by 0.5.** §9.1 "Integers": outside the 64-bit signed range is E000; zk2py's guess stands.

**Status at 0.4: partly resolved by 0.4.** TOML 1.1-only syntax is now E000 (§9.1, five `e000-toml11-*` fixtures), which settles the 1.1 half. The 64-bit integer limit inside TOML 1.0 is still unaddressed.

TOML 1.0 says: "If an integer cannot be represented losslessly [in 64
bits], an error must be thrown." Python's `tomllib` accepts
`9223372036854775808` regardless.

A reader that keeps the value goes on to E028 ("an integer outside
±(2^53−1) in the canonical form"). A reader that follows TOML reports E000
("text that is not TOML"). §9.1 already acknowledges the same kind of
divergence for TOML 1.1 ("a 1.1-only contract can load in Rust and fail in
a 1.0 reader"). The 64-bit limit is a second case, inside TOML 1.0.
**Resolved:** a guess. zk2py refuses integers beyond 64 bits as E000.

### F-17 · ambiguity · `contract.schema.json` and `descriptor.schema.json`, integer bounds

**Status at 0.5: resolved by 0.5.** `contract.schema.json` carries `maximum` on its `uint32` fields, and §9.1 makes `format` a bound for contracts. The descriptor schema's `uint64` leaves F-63.

`major`, `minor`, `deprecated.since` and `history.depth` are
`{"type": "integer", "format": "uint32", "minimum": 0}`. JSON Schema
2020-12 treats `format` as an annotation, so a standard validator accepts
`major = 4294967296`. §9.1 bounds `major` to "0 to 2^32−1".
**Resolved:** a guess. zk2py's shape checker asserts `uint32` and `uint64`,
giving E000 and D000. A `maximum` in the schema would remove the doubt.

### F-18 · ambiguity · §9.1 and E020, "a datetime"

**Status at 0.5: resolved by 0.5.** §9.1: all four TOML date/time kinds, at any depth.

**Status at 0.4: partly resolved by 0.4.** The 0.4 changelog says a time without seconds "E020 would have refused … anyway, as an annotation datetime", so a local time counts. That is said in the changelog, not in §9.1, and nothing names a local date.

> "Annotation values are any TOML value except a datetime (E020)."

TOML has four temporal types: offset date-time, local date-time, local date
and local time. `e020-datetime` uses an offset date-time.
**Resolved:** a guess. All four count, at any depth of the value.

### F-19 · gap · §9.1 and §9.5, floats and the canonical restrictions

**Status at 0.5: resolved by 0.5.** §9.5 defines the canonical domain: `nan`/`inf` and integral floats JCS writes as integers beyond ±(2^53−1) are E028, one per value; `1.0` and `1` fingerprint alike.

> §9.1: "Floats are allowed."

> §9.5: "every integer is within ±(2^53−1). … On that domain every JCS
> implementation agrees."

Three consequences go unaddressed:
1. **`nan` and `inf` are valid TOML.** No lint refuses them, and RFC 8785
   cannot serialize them, so the contract has no canonical bytes.
2. **`1e16` is a float**, so the lints never see "an integer". But JCS
   writes it as the integer literal `10000000000000000`. A verifier parses
   it back as an integer beyond 2^53−1 and refuses the bundle at §9.6 step
   6 (`restrictions`). A contract that lints clean can thus build a bundle
   that no implementation verifies.
3. **`1.0` and `1` have the same canonical bytes.** Changing an annotation's
   type keeps the fingerprint.

**Resolved:** a guess. An integral float beyond ±(2^53−1) is counted as
E028; zk2py stops at 1e21, where ECMAScript switches to exponent form.
`nan` and `inf` are reported as E028 too (`jcs.restriction_violations`,
`contract.canonicalize`).

## §9.2 The lints

### F-20 · ambiguity · E020, counting

**Status at 0.5: resolved by 0.5.** E020 counts a key's value and its name apart (one key can give two), and requirement annotations are checked. zk2py's guess (one per key) was overturned.

> "per key and table; once for `[defaults]`, whatever it reaches"

When a key is malformed *and* its value holds a datetime, is that one E020
or two? Do `[requires.<role>]` annotations count as one of these tables?
§9.1 lists `annotations` among a requirement's fields, but §9.2's E020
does not name them.
**Resolved:** a guess. One E020 per key, and requirement annotations are
checked like any other table.

### F-21 · gap · §10 point 2, Appendix D, W105 without a vocabulary

**Status at 0.5: resolved by 0.5.** §10: a profile with no interim table raises no W105; zk2py's guess stands.

> "Until a profile publishes its vocabulary, the interim tables of
> Appendix D apply, and a key outside them is a warning."

A contract may `use` a profile that has no interim table, such as
`acme.v1`. Is every key of that profile then W105, or none?
**Resolved:** a guess. None.

### F-22 · ambiguity · E018 and E033, written or resolved

**Status at 0.5: resolved by 0.5.** §9.3: lints read resolved values; zk2py's guess stands.

> "E018 | `serving = "replicated"` without `idempotent = true`"

> "E033 | `summary` without `replies = "many"`"

`serving`, `idempotent` and `replies` are defaultable (Appendix D). With
`[defaults.operation] idempotent = true`, is a resource that sets only
`serving = "replicated"` an E018?
**Resolved:** a guess. zk2py judges the resolved values (§9.3).

### F-23 · ambiguity · §9.2, small lint edges

**Status at 0.5: resolved by 0.5.** Retention may have leading zeros, a rate may not; `gate = []` is no gate (zk2py's E016 guess overturned); `history = false` on an event is E019; W103 waits for resolved types (cascade 6). "Fit 64 bits" leaves F-56.

None of these is covered by a fixture:
- **Leading zeros in `retention`.** E026: "`burst(<n>/h)` (n ≥ 1, decimal,
  no leading zero); `retention` not `<n>` + `s`/`m`/`h`/`d`/`w` (n ≥ 1)".
  `07d` is accepted, because the canonical form stores seconds anyway.
- **`gate = []`.** zk2py treats the field as present, so it is E016 without
  `optional = true`.
- **`history = false` on an event.** zk2py reports E019, because the field
  is written.
- **W103 beside E023.** "written on a resource where no JSON Schema type
  takes it" is unknowable when the type does not resolve. zk2py skips W103
  there.

**Resolved:** guesses, as listed.

## §9.6 Bundles

### F-24 · gap · §9.6 step 9, the base64 variant

**Status at 0.5: resolved by 0.5.** §9.6: RFC 4648 §4, padded, strict; zk2py's guess stands.

> "it has a `data` member, base64 text for protobuf"

The prose does not say which alphabet, whether padding is required, or
whether decoding is strict.
**Resolved:** from the fixtures. zk2py's builder reproduces
`valid.bundle.json` byte for byte with RFC 4648 §4 standard, padded base64.
The verifier is strict, so unpadded or URL-safe text is `shape`.

### F-25 · gap · §9.6 steps 7, 9 and 10, entry shapes

**Status at 0.5: resolved by 0.5.** Step 9: a non-object entry or one without `kind` is `schema_kind` (zk2py's `shape` guess overturned). Entry members besides `kind`/`data` are `shape`. An extra's `media_type` must be a string and is informative. One id listed twice is one artifact.

The verification steps leave several shapes open:
- **A schema entry that is not an object.** zk2py: `shape`.
- **A schema entry with no `kind`.** `schema_kind` or `shape`? zk2py:
  `schema_kind`.
- **Unknown members** inside a schema entry or an extra entry, such as a
  stray `"sig"`. Step 3 refuses unknown top-level members, but nothing says
  what happens at this level. zk2py ignores them.
- **An extra's `media_type`.** Shown in the example, never checked. zk2py
  ignores it.
- **The same id listed twice** in `contract.schemas`, for two listed files
  with identical bytes. Not addressed.
- **A JSON Schema `data` with no JCS bytes** (an integer beyond 2^53−1).
  zk2py: `schema_hash`.

**Resolved:** guesses, as listed.

### F-26 · gap · §9.6, extras

**Status at 0.5: resolved by 0.5.** By deferral: where a builder finds extra documents is `views.v1`'s; meanwhile a core builder carries no extras and such a bundle fails step 11. A `views.document` value is one id string (zk2py's list guess overturned), and a bundle always writes all three members.

> "Extras are exactly the documents that the contract's `views.document`
> annotations reference. … The reference builder does not carry extras
> yet"

Open points:
- **No source.** The spec never says where a builder finds the document for
  an id: not a path, a directory, or a convention.
- **The value's shape.** Is a `views.document` value one `sha256:…` or a
  list? Appendix D lists only the key.
- **Which annotations.** Step 11 says "of the contract's resources", which
  leaves out requirement annotations.
- **An empty `extras`.** A built bundle always writes `"extras": {}`. That
  changes the bytes, and §9.6 makes one revision one bundle. Only
  `valid.bundle.json` shows it.

**Resolved:** the builder refuses a contract that uses `views.document`.
The verifier accepts a string or a list of strings. `"extras": {}` is
written, from the fixture.

## §9.7 History and retention

### F-27 · ambiguity · §9.7, the history check

**Status at 0.5: resolved by 0.5.** §9.7 gives the check's order. `interface` and `jcs` are both reported (zk2py's one-per-file guess overturned); `jcs` compares against all three members written.

> "The history check MUST verify: every bundle (§9.6); its fingerprint
> against its file name; its interface against its directory; that it is
> in JCS form. It reports `directory` …, `file_name`, `io`, `interface`,
> `jcs`, or a bundle tag."

Open points:
- **One problem per file, or every problem?** For example, a wrong
  directory *and* not JCS.
- **In which order** are the four checks made?
- **A plain file** at the root of the history.
- **A subdirectory** inside an interface directory.
- **"In directory then file order"** (`history/expect.json`): by bytes, or
  by locale?
- **Is a bad directory's content checked?** `bad-directory` shows it is not.

Each fixture has exactly one problem, so the order is untested.
**Resolved:** a guess, consistent with every fixture:
- the first problem per file, in this order: verify (with the fingerprint
  the file name implies, so `wrong-name` gives the bundle tag
  `fingerprint`), then interface, then JCS;
- entries sorted by name;
- a non-interface entry at the root is `directory`, and its content is
  skipped.

### F-28 · ambiguity · §9.7, retention identity

**Status at 0.5: resolved by 0.5.** §9.7 defines retention identity apart from the classes, with protoc's default `json_name`; zk2py implements it (`compat.identical`).

> "A rebuild that the classifier judges identical to the newest published
> revision keeps that revision's bundle … For protobuf, identity compares
> the `FileDescriptorSet`s with source info dropped and every default
> `json_name` dropped."

Open points:
- **"Identical" is not one of the classifier's classes.** Those are
  compatible, review and breaking.
- **Identity for JSON Schema and raw types** is not defined. Canonical-byte
  equality is the obvious reading.
- **The default `json_name`** is protoc's `ToJsonName` (underscores
  removed, the next letter upper-cased), which the spec does not state.
- **What is compared:** messages or bytes.

**Resolved:** the fixtures decide `same_revision` for the protobuf payload
cases:
- the normalized sets are compared as messages;
- the default `json_name` follows protoc's rule.

Identity for JSON Schema and raw types is not exercised.

## §9.8 Compatibility

### F-29 · ambiguity (fixture decided) · §9.8, "in both directions"

**Status at 0.5: resolved by 0.5.** §9.8 now says a direction is a role, never a swap.

**Status at 0.4: resolved by 0.3.** The tables now list both transitions of a member with different classes (`explicit_set` breaking against `explicit_cleared` review; `idempotent_cleared` against `idempotent_set`; `fanout`, `replies` and `reliability` each way). A rule is therefore a directed transition, and "both directions" can only mean the reader and writer roles. The phrase itself is unchanged.

> "a candidate against every revision in the history, in both directions.
> Each rule is judged per direction: for streams, state, events and
> responses, the owner writes and the consumer reads; for requests, the
> caller writes and the owner reads."

One reading compares old→new *and* new→old. Under it, every directional
row of the table becomes symmetric. `explicit` true → false (review) would
also be false → true (breaking), so breaking overall, but
`compat/contract/explicit-true-to-false` says review. The same reading
would raise a spurious `field_deleted_unreserved` for every added protobuf
field.

**Resolved:** from the fixture. "Direction" means the reader and writer
roles, and every rule's class already accounts for both. zk2py classifies
only earlier → candidate.

### F-30 · gap (fixture decided) · §9.8, pairing resources across revisions

**Status at 0.5: resolved by 0.5.** §9.8: resources pair by kind and template (see F-40).

**Status at 0.4: resolved by 0.3, but contradictorily.** §9.8 now says resources are "matched by kind token and template", which contradicts its own explicit rows and the fixture: see F-40.

The spec never says how two revisions' resources are paired.
- **By `(kind token, template)`:** `explicit` true → false becomes a
  removal plus an addition, so breaking.
- **By template text:** it is the review the fixture expects.

**Resolved:** from the fixture. zk2py pairs by template, which is unique
within a contract because it is the table key.

### F-31 · gap · §9.8, the contract table is not exhaustive

**Status at 0.5: resolved by 0.5.** §9.8: only artifacts a type reaches are compared; an unreferenced one changing is compatible.

**Status at 0.4: partly resolved by 0.3.** The six tables now class every canonical member, in both directions. zk2py follows them, which changed twelve of its guesses (the list is in the README and `zk2py/compat.py`'s docstring). One remainder: a listed artifact that no type references can change with no rule firing (zk2py: compatible, because nothing reads it).

The table gives no class for any of these changes:
- **Resources:**
  - a resource *added*;
  - a resource's `kind` changed;
  - its `params`, `cardinality`, `epoch`, `gate`, `history`, `rate`,
    `retention_s`, `timeout_ms`, `serving`, `encoding`,
    `attachment_encoding`, `annotations` or `deprecated` changed.
- **The reverse transitions:**
  - `idempotent` false → true;
  - `fanout` forbidden → allowed;
  - best_effort → reliable;
  - block → drop;
  - many → one;
  - required → optional.
- **Roles:**
  - a role *removed*;
  - optional → required;
  - its `resources` or `annotations` changed.
- **Uses:** `uses` changed.
- **Types:**
  - a type changing kind (raw → jsonschema);
  - a raw media type changed;
  - an `error` or `attachment` type added or removed.

**Resolved:** guesses. Every unlisted change is **review**, with these
exceptions:
- a `kind` change, a type changing kind, and a raw media-type change are
  breaking;
- an optional resource added is compatible, by analogy with "An optional
  role added";
- a role going optional → required is breaking, by analogy with "A
  required role added".

### F-32 · gap (partly fixture decided) · §9.8, the JSON Schema rules are not exhaustive

**Status at 0.5: resolved by 0.5.** §9.8: `$ref` siblings are merged into the target and compared (zk2py's review guess overturned); a boolean schema change is review; a `required` name without a property is breaking.

**Status at 0.4: partly resolved by 0.3.** §9.8 now states `type_changed` for any type set change, `required_removed`, `const_changed`, the `additionalProperties`/`items` toggles (compatible) and schema gains (review), and enum reordering (compatible). Still unlisted: a `$ref` beside other keywords, a boolean schema in a property position, and a `required` name with no property.

The rules list "integer ↔ number" as the only type change.
`compat/transitive/jsonschema-property-retyped` needs `string → integer` to
be breaking (v3 against v1), and only that fixture says so.

There is also no rule for:
- a *required* property removed;
- `const` changed;
- `items` changed;
- `additionalProperties` toggled between `true`/absent and `false`, or
  between a boolean and a map schema;
- a `required` name with no matching property;
- a `$ref` beside other keywords.

**Resolved:** the fixture decides that any `type` change is breaking. The
rest are guesses:
- a required property removed, `const` changed and `items` changed are
  breaking;
- `additionalProperties` boolean toggles are compatible ("Readers tolerate
  unknown properties");
- boolean ↔ schema and a `$ref` beside other keywords are review.

### F-33 · ambiguity · §9.8, `oneOf` versus `anyOf`

**Status at 0.5: resolved by 0.5.** §9.8: `oneOf`/`anyOf`/`prefixItems` compare as written, in order, so a reordering is review (zk2py's multiset guess overturned); "a branch added" is more branches; the asymmetry is deliberate. An absent `oneOf` leaves F-60.

> "Breaking: … a `oneOf` branch added." "Review: any other change inside
> `oneOf`, `anyOf` or `prefixItems`."

Two questions follow:
- **Branch identity.** Is it textual? Does order or an annotation count?
- **The asymmetry.** An `anyOf` branch added and a `oneOf` branch
  *removed* are only review. That is intended by the text, but stated
  nowhere as intended.

**Resolved:** a guess. Branches compare as a multiset of normalized schemas,
with annotations dropped and `$ref`s inlined. "Added" means the old
multiset is a strict subset of the new one. `prefixItems` compares in
order.

### F-34 · contradiction · §9.8 against `compat/payload/protobuf/renumber-field`

**Status at 0.5: resolved by 0.5.** The renumber bullet no longer calls a renumber a deletion.

**Status at 0.4: resolved by 0.3.** §9.8 now says "Fields are matched by number. A field missing by number but present by name is renumbered": the same field is the same name, and a renumbered field is not a missing one, so it raises no warning. The bullet still calls a renumber "a deletion plus an addition", which is what made the old reading possible.

> "it is renumbered: a deletion plus an addition of the same field"

> "Warning `field_deleted_unreserved` (reported alongside the class): a
> field deleted without reserving its number."

`renumber-field` moves `string frame` from 3 to 10 without reserving 3. By
the prose's own definition that includes "a deletion" of a field without
reserving its number. Yet `expect.json` gives `"warnings": []`.

The prose also does not say what "the same field" means: the same name,
the same name and type, or something else.
**Resolved:** from the fixture. A renumbered field (same name, new number)
is breaking, and its deletion raises no warning.

### F-35 · gap · §9.8, the protobuf rules are not exhaustive

**Status at 0.5: resolved by 0.5.** §9.8: structure from the named type only (nested types no field reaches are not compared); oneofs by name; enums by number, closed by the candidate's syntax alone; proto2 defaults (zk2py's review guess overturned), options and reserved names not compared; warnings deduplicated.

**Status at 0.4: partly resolved by 0.3.** §9.8 now states presence (`presence_changed`), map cardinality, `reserved_reused`, proto2 `required` added/removed/toggled, and that "nested and referenced" messages are compared "each pair once". Still open: proto2 defaults, options such as `packed`, an enum's openness when the two revisions' syntaxes differ, a field moved *between* oneofs, nested types added or removed, reserved *names* reused, and how warnings accumulate over a history.

None of these is covered:
- a proto3 `optional` toggled, which is wire-compatible but adds a
  synthetic oneof;
- a field moved *between* two oneofs;
- a proto2 `default` changed;
- options other than `json_name` (`packed`, `deprecated`);
- a message-typed field whose type is renamed but keeps the same
  structure;
- which types are compared: the root only, or every type reachable from
  it;
- an enum value renumbered;
- a proto3 message that uses a proto2 enum, which is open or closed;
- how `field_deleted_unreserved` accumulates over a history: once per
  earlier revision, or deduplicated.

**Resolved:** guesses:
- types are compared structurally from the root, through fields;
- a proto3 `optional` toggle is review;
- a move between oneofs is breaking (by name);
- a default change is review;
- options other than `json_name` are ignored;
- an enum is closed if either revision's file is proto2;
- warnings are listed once per deleted field per comparison.

### F-36 · gap · `compat/` and Appendix E, the input layout

**Status at 0.5: resolved by 0.5.** Resolved by 0.3; nothing more.

**Status at 0.4: resolved by 0.3.** `compat/README.md` now gives the case layout, the one-resource wrapper and the meaning of each `expect.json` member (but see F-44 on the wrapper's location).

Appendix E says only "`compat/`: the class, warnings and `same_revision`".
The rest is learned from the files:
- old/new directories against `v1…vN` with `history` and `candidate`;
- payload cases are bare schema files or `.proto` directories with no
  contract;
- the protobuf file is always `m.proto`, and its import root is the
  revision directory;
- `type` names the type to compare;
- an invalid revision is detected by loading the file alone.

**Resolved:** from the fixture files.

## Fixtures and documents

### F-37 · gap · `conformance/README.md` and fixture descriptions: meanings deferred to Rust and design documents

**Status at 0.5: resolved by 0.5.** Fixture documents cite spec sections; only a provenance line naming `zenkey-model` remains, which defines nothing.

The spec claims to stand without the reference implementation. Its fixture
documents point at it, and at design documents, for meanings:

| Pointer | Where | What it names |
|---|---|---|
| `zenkey_model::diag::CODES` | conformance/README.md, contracts/expect.json | the meaning of codes |
| `zenkey_model::bundle::BundleError::tag` | bundles/expect.json | refusal tags |
| `check()` | descriptors/expect.json | the descriptor checker |
| `check_set` | sets/expect.json | the set checks |
| `r3.3 D1` | templates.json, conformance/README.md | design documents |
| `r3 §3.11` | conformance/README.md | design documents |
| `docs/zk2/spike-results/s7/` | compat/expect.json | spike results |

For E/W codes and bundle tags, §9.2 and §9.6 were enough. For the D codes
they were not (F-04).
**Resolved:** nothing to implement. Reported because each pointer is an
invitation to cross the information barrier.

### F-38 · gap · §9.2 and `sets/`, set loading

**Status at 0.5: resolved by 0.5.** §9.2: a set check runs only when every member loads, in file-name order; E035 against the first declaration. zk2py's "refuse the set" guess was overturned: a failing member's own codes stand.

> "E035 and E036 are set checks: each file is loaded on its own first."

Only `sets/expect.json` adds "and must load". The spec does not say what a
set check reports when a member does not load. Nor does it say which
declaration E035 checks a requirement against when E036 reports two
contracts declaring the same interface.
**Resolved:** a guess. A member that does not load refuses the whole set
check. E035 uses the first declaration in file-name order.

### F-39 · ambiguity · §9.2 E021 and E022, "after the first"

**Status at 0.5: resolved by 0.5.** §9.2 "Order": template order, not document order (zk2py's guess overturned); E021 does not take a resource out of W101.

> E021: "once per template after the first of its shape"; E022: "once per
> `epoch` template after the first"

TOML 1.0 does not define an order for a table's keys, and resources are
table keys. The *codes* are unaffected, since they are counts. Which
resource carries the error is affected, and so is W101's "resources without
errors" (cascade 4). A different "first" can change which pairs W101
considers.
**Resolved:** a guess. Document order, as Python's `tomllib` and most
readers preserve it.

## New at 0.4 (after amendments 0.3 and 0.4)

### F-40 · contradiction · §9.8 "Resources" against `compat/contract/explicit-true-to-false`

**Status at 0.5: resolved by 0.5.** §9.8 pairs by kind (not kind token) and template; a kind change is `resource_removed`.

> "**Resources,** matched by kind token and template:" … "`explicit` false
> → true | breaking for ambient consumers | `explicit_set`" … "`explicit`
> true → false | review (link budgets) | `explicit_cleared`"

Toggling `explicit` changes a resource's kind token (`stream` ↔ `@stream`,
`state` ↔ `@state`, §1.3). Paired "by kind token and template", the old
and new resources never pair, and `explicit_cleared` can never fire:
- `explicit` true → false would be `resource_removed`, which is breaking;
- `explicit-true-to-false` expects review.

The table also names `token_changed` beside `kind_changed`. Under that
pairing a token cannot change between paired resources either.

**Resolved:** from the fixture. zk2py pairs resources by template, which is
unique within a contract because it is the table key, and compares tokens
inside the pair. This replaces F-30, whose silence 0.3 filled with this
sentence.

### F-41 · ambiguity · §9.1 (0.4): "MAY accept later TOML" against "is E000"

**Status at 0.5: resolved by 0.5.** §9.1: the list is closed, and a reader "MAY be a parser of later TOML, provided it reports these as E000".

> "A reader MAY accept later TOML, but a contract MUST NOT need it, so
> syntax that only TOML 1.1 has is **E000**:" (five constructs follow)

> CHANGELOG 0.4: "A reader MAY still accept later TOML. The rule binds what
> a contract needs, not what a reader parses."

Three things are unclear:
- **"Accept".** A 1.1 reader may parse these constructs, yet must still
  report E000 for them. So "accept" can only mean "parse, then refuse",
  which is not what the word usually says.
- **Is the list closed?** It reads as the whole TOML 1.1 delta, but §9.1
  does not say so. Is it the rule ("syntax that only TOML 1.1 has"), or
  are these five constructs the rule?
- **Later versions.** A future TOML 1.2 construct is not covered by
  either reading.

zk2py's E000 for these constructs is Python's `tomllib` refusing them,
because it is a strict 1.0 reader. A later `tomllib` that reads 1.1 would
silently stop reporting them. zk2py therefore probes its reader with the
five constructs at load, and refuses to run if any parses
(`contract._require_toml_1_0_reader`).

**Resolved:** the five fixtures pass. The guard is zk2py's own choice.

### F-42 · contradiction (documentary) · CHANGELOG 0.3 and `compat/expect.json`'s description

**Status at 0.5: resolved by 0.5.** Corrected: CHANGELOG 0.3 now reads 23 new, and `compat/expect.json`'s description no longer says inputs are only loaded.

Two statements are stale:
- **CHANGELOG 0.3:** "`compat/` is evaluated. 70 cases, 11 of them new."
  Against 0.2's 47 cases, 23 are new:
  - 12 contract cases;
  - 5 JSON Schema cases;
  - 6 protobuf cases.
- **`compat/expect.json`'s `description`** still says "Evaluated by the
  classifier (#618); until it exists, the runner checks that every input
  loads." 0.3 removed exactly that caveat from §0 ("§0's caveat on
  `[F: compat/]` is removed").

**Resolved:** nothing to implement. Reported here.

### F-43 · ambiguity · §9.8 protobuf, `presence_changed`

**Status at 0.5: resolved by 0.5.** §9.8: presence is protobuf's definition, and a oneof move reports `oneof_changed` alone (zk2py's two-rule guess overturned).

> "explicit presence toggled, such as proto3 `optional`
> (`presence_changed`): a reader stops telling a default from an absent
> value"

The spec does not define which fields have explicit presence. The usual
protobuf definition gives presence to:
- every proto2 singular field;
- proto3 `optional` fields;
- message-typed fields;
- oneof members.

Under that definition, two more changes toggle presence besides the
`presence-toggled` case:
- moving a proto3 scalar into a oneof, which is already `oneof_changed`
  (breaking). Does it also raise `presence_changed`?
- changing a file's `syntax` between proto2 and proto3.

The class is unaffected, since the worst wins. Only the reasons a
classifier reports differ.

**Resolved:** a guess. zk2py uses the definition above
(`compat._presence`), so `move-into-oneof` reports both rules.

### F-44 · ambiguity · `compat/README.md`: the wrapper's location, and `invalid`

**Status at 0.5: resolved by 0.5.** `compat/README.md`: the wrapper sits beside its artifact; only a candidate can be `invalid`.

> "`payload/protobuf/<case>/` | `old/m.proto`, `new/m.proto` | Each wrapped
> in a one-resource contract (below) … with `<kind>` `protobuf` or
> `jsonschema` and `<artifact>` the case's file"

Two points are open:
- **Where the wrapping contract sits.** It is not stated, and it decides
  the protobuf artifact's import root, and with it the
  `FileDescriptorProto.name`.
  - A wrapper at the case root, listing `old/m.proto` and `new/m.proto`,
    gives two different file names. `doc-only`'s `same_revision` is then
    `false`, against `true` expected.
  - Only a wrapper inside each revision's directory, listing `m.proto`,
    reproduces the fixture.
- **When a case is `invalid`.** "`invalid` when the new revision must not
  load (its only error is E037)" leaves open what happens when the old
  revision, or a history revision of a transitive case, does not load, and
  when the new one fails with another error.

**Resolved:** the fixture decides the location. zk2py places the wrapper in
the revision's directory for protobuf, and in the case directory for JSON
Schema. For `invalid`, the guess is that any revision of the case failing
to load makes it `invalid`.

### F-45 · gap · §9.7 and `examples/zk2/.history`: where a contract's history lives

**Status at 0.5: resolved by 0.5.** §9.7: a configured history root, a contract's history found by interface id; `examples/zk2/README.md` states the examples' requirement.

> "A contract's CI keeps every published bundle at
> `contracts/.history/<iface>/<hex>.bundle.json`"

`examples/zk2/` keeps one `.history` root for contracts in four
subdirectories (`walkthrough/`, `tcgui/`, `zenoh-modem/`, `zensight/`). The
spec does not say how a tool finds the history of a given contract file:
- the nearest ancestor `.history`;
- a configured root;
- a sibling of `contracts/`.

The requirement that every example be published there, and be compatible
with its history, is not written in `spec/` or in `examples/zk2/README.md`.
It came to zk2py from the coordinator of #609.

**Resolved:** a guess. zk2py's examples check uses `examples/zk2/.history`
for every example. All 24 are:
- published there;
- byte-identical to the bundle zk2py builds, protobuf ones included;
- `compatible` against their history.

## The live half, first slice (against 0.4; #609, #610)

All ten are resolved by 0.5; their status is in the summary table at the
top, and on each entry.

These come from `zk2py.live` and `zk2py.live_interop`, which run the Rust
owner example as a black box over zenoh-python 1.10.1. The setup: loopback,
the owner as the router, and zk2py as a client of it. The rules read are
§3.3, §8.1–§8.4, `scenarios/presence.md` and `scenarios/retrieval.md`. Two
entries rest on measurements, which are given.

| Id | Severity | Location | In one line |
|---|---|---|---|
| F-46 | gap | §3.3 | The descriptor GET: no target, consolidation, timeout or reply encoding is stated. |
| F-47 | gap (measured) | §8.1 "Reading presence", presence.md §4 | The handler rule binds only the GET. A bounded, undrained *subscriber* on the same session starves even a callback GET, which then returns 257 of 2,002 tokens at its timeout. |
| F-48 | gap | §8.4, §9.6 | The bundle reply's encoding is not stated. |
| F-49 | gap | §8.1, §3.3, §8.4 | No timeout anywhere: liveliness GET, descriptor GET, each §8.4 attempt, waiting for presence. |
| F-50 | gap (measured) | §8.4 step 2 | "Verify each reply as it arrives" needs consolidation `None`. With zenoh's default, a slow corrupt holder's reply arrives alone, at completion, and the valid bundle is consolidated away. |
| F-51 | ambiguity | §8.4 step 1, retrieval.md §1 | "The nearest holder on each router": from a client that holds a bundle itself, `BestMatching` returned two replies. |
| F-52 | gap | §3.1, §3.2 R1, §8.2 | Nothing says an owner with an unbound *required role* must not start, yet the reference owner refuses to, citing R1. |
| F-53 | ambiguity | §3.3, §10 point 4 | Is a descriptor's `profiles` the union of its contracts' `uses`? |
| F-54 | gap | §8.1 member tokens | When does a member exist, and so need its token? |
| F-55 | ambiguity | §3.2 R3 | Is an unconfigured optional role listed with `bindings: []`, or omitted? |

### F-46 · gap · §3.3, the descriptor GET

**Status at 0.5: resolved by 0.5.** §3.3 "The GET": one reply, `application/json`, no timestamp, no attachment, consolidation `None`; the target is the caller's.

> "Every instance serves a **descriptor**: a JSON document answered on GET
> at its instance key, and put on every change."

The GET's target, consolidation and timeout are not given. Nor is the
reply's `Encoding`: §7.2's encoding rule is about resource samples, and
§5.2's about the error envelope. Nor is whether the reply carries a
timestamp (S1–S2 bind state, not control keys), or how many replies to
expect.

The owner answered with one reply, encoded `application/json`, with no
timestamp and no attachment.
**Resolved:** a guess:
- zk2py GETs with `BestMatching`, consolidation `None` (see F-50) and 5 s;
- it discards a reply on a key that is not concrete (R6);
- it treats `application/json` as an expectation the spec does not state.

### F-47 · gap (measured) · §8.1 "Reading presence" and `presence.md` §4

**Status at 0.5: resolved by 0.5.** §8.1: every liveliness subscriber on the session MUST be callback-driven or drained; a GET that ended at its timeout SHOULD be read as possibly incomplete; presence.md §4 now states zenoh-python's measurement.

> "A caller or tool's liveliness GET on a session that holds a liveliness
> subscriber MUST use a callback or an unbounded handler. With zenoh's
> default 256-slot handler, such a GET hung at every measured size from
> 996 tokens (zenoh#2678)."

Measured with zenoh-python 1.10.1:
- **Setup:** 2,000 extra instance tokens, declared by a second client. The
  reading session holds a liveliness subscriber on `zk2/*/*/@zk/**`, and
  issues `liveliness().get("zk2/*/*/@zk/**", timeout=10)`.
- **Total:** 2,002 tokens, counting the owner's two.

| GET handler | the session's liveliness subscriber | result |
|---|---|---|
| `Callback` | `Callback` | complete: 2,002 in 0.34 s |
| default, drained as replies arrive | `Callback` | complete: 2,002 in 0.43 s |
| default, drained after 3 s | `Callback` | complete: 2,002 in 3.0 s |
| default | default, `history=True`, never drained | **hung**: 0 replies after 20 s |
| `Callback` | default, `history=True`, never drained | **257 of 2,002**, ended at the 10 s timeout |

Three things follow:
1. **The scenario did not reproduce as written.** `presence.md` §4's "With
   zenoh's default 256-slot handler, it hangs" did not happen for the GET's
   handler alone in zenoh-python.
2. **What starves the GET is the subscriber's handler,** when it is bounded
   and nobody drains it. Then even a callback GET is cut short, silently,
   at its timeout. A tool that trusts it under-reports presence, which is
   exactly what O5 and R7 warn against.
3. **zenoh-python has no unbounded handler.** Its default handler and
   `FifoChannel` are bounded, and `RingChannel` drops. So "an unbounded
   handler" is not an option there; only a callback is.

The rule should also bind the subscriber's handler. A tool should treat a
liveliness GET that ended at its timeout, rather than at the routers' final
reply, as possibly incomplete.

**Resolved:** how zk2py meets the rule:
- every liveliness GET passes `zenoh.handlers.Callback(on_reply, on_done)`
  (stable API, default `indirect` mode). The callback only appends to a
  Python list, and `on_done` marks completion;
- the session holds no bounded subscriber. The scale check's subscriber is
  a `Callback` too;
- `list_presence` reports `complete=False` when the GET ended at its
  timeout.

The runner checks the §4 scenario with callbacks: 2,000 of 2,000 tokens in
about 0.15 s while a subscriber is held.

### F-48 · gap · §8.4 and §9.6, the bundle reply's encoding

**Status at 0.5: resolved by 0.5.** §8.4: one reply, `application/json`, and a caller MUST NOT depend on the encoding.

§9.6 defines the bundle *bytes* (JCS). §8.4 defines how to retrieve and
verify them. Neither says what `Encoding` a holder sets on its reply. The
owner sets `application/json`.
**Resolved:** zk2py does not depend on it. Verification is by hash, as §8.4
intends ("the hash is the check"). zk2py records the encoding in its report.

### F-49 · gap · timeouts: §8.1, §3.3 and §8.4

**Status at 0.5: resolved by 0.5.** §8.1: timeouts are the caller's; a conformance run uses 1 s. The starting instant leaves F-59.

The spec sets no timeout for any of these:
- a liveliness GET;
- the descriptor GET;
- each of §8.4's two attempts. "If none was valid, retry once" depends on
  when the first attempt counts as done;
- how long a tool waits for presence after an owner starts.

S6 speaks of "the GET's timeout" as if it were given.
**Resolved:** a guess:
- 5 s per GET;
- 2 s for the attempts the runner expects to fail;
- 30 s to wait for presence, polled every 0.2 s.

### F-50 · gap (measured) · §8.4 step 2 and consolidation

**Status at 0.5: resolved by 0.5.** §8.4: consolidation `None` is a MUST on both attempts, with the measurement.

> "Verify each reply **as it arrives** (§9.6), and accept the first valid
> one, without waiting for the GET to complete."

§4.1 says `Latest` consolidation "delivers at query completion", and
zenoh's default for a GET is `Auto`, which is `Latest` on a concrete key.
§8.4 never says to set consolidation. O2 and O6 do say it, for calls.

Measured: on the `nav.v2` contract key, zk2py's session held a holder that
answers a corrupt 12-byte bundle after 2 s, and the owner's holder answers
the valid 4,800-byte bundle at once:

| Consolidation | Target | Replies delivered (arrival in s, size in bytes) |
|---|---|---|
| default (`Auto`) | `BestMatching` | (2.001, 12) |
| default (`Auto`) | `All` | (2.000, 12) |
| `None` | `BestMatching` | (0.001, 4800), (2.000, 12) |
| `None` | `All` | (0.001, 4800), (2.000, 12) |

With the default, the valid bundle never arrives: it is consolidated away,
and only the corrupt one is delivered, at completion. A tool that follows
§8.4 to the letter, with zenoh's defaults, waits 2 s, refuses the corrupt
reply, retries with `All`, and reports the contract unavailable while a
valid holder answers in 1 ms.

**Resolved:** a guess. zk2py sets consolidation `None` for every §8.4 GET,
and for the descriptor GET. §8.4 should say so.

### F-51 · ambiguity · §8.4 step 1 and `retrieval.md` §1, "nearest holder"

**Status at 0.5: resolved by 0.5.** §8.4 step 1: a holder on the caller's own session answers too; assume nothing about the count.

> "GET with target `BestMatching`. That reaches the nearest holder on each
> router the query visits."

> retrieval.md §1: "With 200 equal holders on one router and
> `BestMatching`, one reply."

Measured in the same setup as F-50: when zk2py's own client session held a
complete queryable on the contract key, a `BestMatching` GET from that
session returned **two** replies, its own and the owner's (see F-50's
table).

So the querying session's own holder counts as a holder of its own.
"Nearest" is the routers' choice, not one a client can make or observe. The
spec does not say how many replies a tool may get from `BestMatching`.
**Resolved:** zk2py assumes nothing about the count. It verifies every reply
that arrives, and accepts the first valid one. The runner's
corrupt-nearest-holder check passes either way.

### F-52 · gap · §3.1, §3.2 R1, §8.2: an unbound required role

**Status at 0.5: resolved by 0.5.** §3.2: an owner whose configuration binds a required role to nothing MUST NOT start. Observing that from outside leaves F-61.

The owner example refused to start on `examples/zk2/walkthrough/thruster.v1.toml`.
It exited with status 1 and declared no instance token, saying:
`NotExposed("role \"cmd\" (twist_cmd.v1) is required and the configuration
binds it to nothing (R1)")`.

R1 says: "a role MUST be bound by configuration, never in code".
- **Required resources:** §8.2 validates that every required *resource* is
  exposed, and `presence.md` §2 step 3 says such an owner "does not start".
- **Required roles:** nothing says an owner with an unbound required
  *role* must not start.

R5 ("A binding resolves at once") and R7 ("A binding MUST NOT require
presence") suggest bindings never block start-up. The owner's documented
contract ("it implements every contract given") does not hold for this
contract either.

**Resolved:** the runner uses only contracts the owner accepts. In place of
`thruster.v1` it uses `camera.v1`, also protobuf. The spec should say what
an unbound required role does.

### F-53 · ambiguity · §3.3 and §10 point 4, the descriptor's `profiles`

**Status at 0.5: resolved by 0.5.** §3.3: `profiles` is the union of the contracts' `uses`, sorted and deduplicated; not checked by the checker.

> descriptor.schema.json: "The profiles this instance follows, as
> `<name>.v<major>`."

The owner listed the union of its contracts' `uses`. For example,
`["freshness.v1", "telemetry.v1"]` for `zk2py_probe.v1` with `zs.snmp.v1`.
Nothing says whether `profiles` must equal, include, or be independent of
those `uses`.
**Resolved:** zk2py does not check `profiles` beyond D010's syntax.

### F-54 · gap · §8.1 member tokens: when does a member exist?

**Status at 0.5: resolved by 0.5.** §8.1: a member exists from the owner's first declaration of it; no member, no member token.

> "The owner MUST hold one member token per member, and cycle it whenever
> that member's continuity breaks."

The spec does not say when a member comes into being: when its first value
is published, when it is configured, or when its device appears.
`zs.snmp.v1` and `zk2py_probe.v1` both have an `epoch` template. The owner,
which publishes no data, held no member token for either.
**Resolved:** zk2py reports the member-token count. It asserts nothing.

### F-55 · ambiguity · §3.2 R3, an unconfigured optional role

**Status at 0.5: resolved by 0.5.** §3.2/§3.3: an unbound optional role is listed with `"bindings": []` and `"params": {}`.

> "**Owner:** the descriptor (§3.3) MUST list every requirement with its
> bindings and parameter bindings as configured."

The owner listed `zk2py_probe.v1`'s optional, unbound role as
`{"role": "upstream", "interface": "nav.v2", "declared_by":
"zk2py_probe.v1", "bindings": [], "params": {}}`.

"As configured" could also be read as "only the roles the configuration
binds". With that reading, an unbound optional role would be omitted, and
the data-flow graph (R3's "read from descriptors") would lose an edge the
contract declares.
**Resolved:** a guess. zk2py's runner requires every contract-declared role
to be listed. The owner passes.

## New at 0.5 (against amendment 0.5)

Found while making zk2py follow 0.5: `just py-conformance` passes 485 of
485, and `just py-live` 60 of 60. Five entries follow up text that 0.5
added. F-62 is a contradiction between two 0.5 rules, and was reproduced.
No fixture pins any of these.

### F-56 · ambiguity · §2.6 and E026 (0.5): "the seconds MUST fit 64 bits"

**Status at 0.6: resolved by 0.6.** §2.6/E026: the seconds are unsigned; above 2^64−1 is E026, and 2^63 s is E028 in the canonical form. zk2py's guess was the rule.

> "In a retention, `<n>` is decimal digits, leading zeros allowed, and at
> least 1 … The seconds MUST fit 64 bits (E026) and, like every canonical
> integer, ±(2^53−1) (E028)."

The spec does not say whether 64 bits means signed or unsigned. A retention
of `9223372036854775808s` (2^63) fits an unsigned 64-bit integer but not a
signed one, so it is either E028 alone or E026 and nothing else (E026 stops
the canonical form). Elsewhere 0.5 bounds integers both ways: TOML's are
signed (§9.1), and a descriptor's `minor` goes to 2^64−1 (§3.3).
**Resolved:** a guess. zk2py uses unsigned: E026 above 2^64−1, then E028
from the canonical form.

### F-57 · ambiguity · §3.3's D008 and D010 (0.5): "once for the repeat"

**Status at 0.6: resolved by 0.6.** §3.3: a repeat is one D008 or D010 for the whole list, and a malformed value counts at each occurrence. zk2py's guess (one per repeated value) was overturned.

> D008 "a capability is not `[a-z0-9][a-z0-9_.-]*`; or one is listed twice
> | per capability; once for the repeat"; D010 likewise for profiles.

Two readings, and the fixtures (`d008-twice`, `d010-twice`) have a single
repeated value, so they cannot tell them apart:
- once per repeated *value*: `["a", "a", "b", "b"]` gives two;
- once for the descriptor, however many values repeat.

Also open: a malformed value listed twice. Is that two D008 for the
malformed occurrences plus one for the repeat?
**Resolved:** a guess. zk2py counts once per repeated value, plus once per
malformed occurrence (`descriptor._repeats`).

### F-58 · ambiguity · §5.2, CBOR (0.5): "an integer outside 64 bits"

**Status at 0.6: resolved by 0.6.** §5.2: a CBOR integer decodes from −2^63 to 2^64−1. zk2py's guess was the rule.

> "A map key that is not text, an integer outside 64 bits, or a float that
> is not finite is `decode`."

CBOR's major type 0 reaches 2^64−1, and major type 1 reaches −2^64. "Outside
64 bits" could mean outside `i64`, outside `u64`, or outside both. 2^63 and
−2^63−1 fall differently under each reading. No case in `errors/cases.json`
has such an integer.
**Resolved:** a guess. zk2py refuses only what neither `i64` nor `u64`
holds, that is, below −2^63.

### F-59 · ambiguity · §8.1 (0.5): when "after an owner starts" begins

**Status at 0.6: resolved by 0.6.** §8.1: the wait starts at the later of the tool's session connecting and the owner's launch, which is the connection when the owner is the tool's router. zk2py's guess was the rule.

> "how long a tool waits for presence after an owner starts, are the
> caller's choices. The scenarios, and so a conformance run, use 1 s unless
> they say otherwise."

A tool cannot observe "starts". The second could be counted from at least
four instants:
- the process launch;
- the owner's router accepting connections;
- the tool's session connecting;
- the owner's last start-up step (§8.2).

The figure is tight enough that the choice matters: process start-up alone
can take a good part of a second.
**Resolved:** a guess. zk2py counts the 1 s from its client session's
connection to the owner's router. The owner example's presence appeared
within it in every run (2 and 4 tokens, in about 1 ms).

### F-60 · ambiguity · §9.8 `oneof_branch_added` (0.5): a side with no `oneOf`

**Status at 0.6: resolved by 0.6.** §9.8: `oneof_branch_added` needs a `oneOf` on both sides; adding or removing the keyword is `undecided_changed`. zk2py's guess (breaking) was overturned.

> "a `oneOf` branch added (`oneof_branch_added`): the candidate's `oneOf`
> has more branches than the earlier one's, whatever they hold."

The rule is open when the earlier revision has no `oneOf` at that position.
Adding a `oneOf` keyword could be:
- `oneof_branch_added`, reading the absent `oneOf` as zero branches; or
- `undecided_changed`, "any other change inside" it.

`oneof-add-branch` has a `oneOf` on both sides.
**Resolved:** a guess. zk2py reads an absent `oneOf` as zero branches, so
adding one is breaking. Removing one is review.

### F-61 · gap · §3.2 and `presence.md` §2 step 4 (0.5): observing a refusal

**Status at 0.6: resolved by 0.6.** §3.2 and presence.md §2 now watch a refusal through a router R1 that outlives the owner, against a control. The Rust owner example is still its own router, so its refusal check still rests on silence. zk2py's own owner, as a client of R1, now follows the scenario in full (`run_python_refusal`).

> §3.2: "An owner whose configuration binds a required role to nothing MUST
> NOT start, as one missing a required resource does not (§8.2 step 2): no
> instance token appears."

The live runner starts the owner on `walkthrough/thruster.v1`. The owner
example is its own router: it prints `listening`, finds the unbound role,
and exits with status 1. zk2py's client completed no presence GET before
the router was gone. So "no instance token appears" was judged on silence,
which O5 says is never a verdict, plus the missing `ready` line and the
exit.

The spec gives no observable for a refusal when the refusing participant
is also the only router. A scenario could run the owner as a client of a
separate router, where a watcher sees no token.
**Resolved:** the runner's check is named for what it observes ("no
instance token while it ran, no `ready` line"), and reports how many
presence GETs it completed: 0.

### F-62 · contradiction · §9.6 against §9.4 (0.5): one id for identical files

**Status at 0.6: resolved by 0.6.** §9.4: a later listed file with an earlier one's id is E024 and is not loaded; the classifier reads a dangling `$ref` as `schema_unreadable`. zk2py follows both.

> §9.6: "Two listed JSON Schema files with identical bytes have one id, so
> the canonical form lists it once, under the `name` of the last of them in
> `[schemas]` order."

> §9.4: "In a bundle, which keeps no paths, the file part names the
> artifact whose `name` is the stem of its last path segment. Stems are
> unique per contract, so this is the file the path named."

The two rules meet in one contract, which zk2py reproduced:
- `[schemas] jsonschema = ["a.json", "b.json", "c.json"]`, with `a.json`
  and `b.json` byte-identical;
- `c.json` holding `{"$ref": "a.json#/$defs/X"}`.

In the source tree the `$ref` resolves by path, so the contract lints clean
(no code). The canonical form lists the shared id once, as `b`. In its
bundle, the file part `a.json` names stem `a`, which no artifact has: the
`$ref` resolves to nothing. §9.8 follows `$ref`s "by stem as in a bundle",
so the classifier cannot see what `c.json` refers to. "So this is the file
the path named" is false here.
**Resolved:** a guess. zk2py follows both rules as written, and so
reproduces the dangling `$ref`: building that contract and reading its
bundle back gives the stems `b` and `c` only. Either rule could yield, for
example:
- list a shared id under every name;
- refuse identical listed files (an E024-like code);
- resolve a bundle file part by *any* name the id was listed under.

### F-63 · ambiguity · `descriptor.schema.json`, §3.3 and §9.1 (0.5): the `uint64` bound

**Status at 0.6: resolved by 0.6.** §3.3: `format` is a bound in a descriptor too, `uint64` being 0 to 2^64−1. zk2py's guess was the rule.

> §9.1: "The schema's `format` is a bound here, not an annotation: `uint32`
> is 0 to 2^32−1, and `uint64` is 0 to 2^63−1, TOML's own bound."

That sentence is scoped to the authoring format ("here").
`descriptor.schema.json` uses `"format": "uint64"`, with no `maximum`, for
`minor` and for every `cardinality` value. §3.3 gives `minor` "0 to
2^64−1". It bounds `cardinality` only by the contract's value (D007). So
whether a descriptor whose `cardinality` value is 2^64 is D000 (outside
`uint64`) or D007 (above the contract's bound) depends on whether `format`
is a bound for descriptors too.
**Resolved:** a guess. zk2py's shape checker bounds `uint64` to 0..2^64−1
in both schemas, so such a value is D000. For contracts this changes
nothing, since TOML cannot write a larger integer.

## New at 0.6: state, operations, and being an owner (#609)

These come from the rest of #609's live scope:
- `zk2py.live.get_state` and `zk2py.live.call`;
- `zk2py.owner`, a minimal owner;
- `zk2py.live_interop`, which reads the Rust owner example's state and
  calls its operations, and lets the Rust `consume` example read zk2py's
  owner.

`just py-live` passes 113 of 113. Each entry below is a rule the
implementation had to guess.

### F-64 · ambiguity · §5.1 O1–O3: the consolidation of a single-reply call

**Status at 0.7: resolved by 0.7.** §5.1 O1: a concrete call MUST set `BestMatching` and `None`. zk2py's guess was the rule.

O2 gives a fan-out call target `All` and consolidation `None`, and O6
gives a `replies = "many"` call consolidation `None`. O1 implies target
`BestMatching` for a concrete call. Nothing names the consolidation of the
common case, a concrete call to a `replies = "one"` operation.

zenoh's default (`Auto`, `Latest` on a concrete key) holds the reply until
the query completes (§4.1), the effect §8.4 measured and forbade for
retrieval. It also ranks an unstamped reply, which every operation reply
is, lowest.
**Resolved:** a guess. zk2py's `call` uses `None` for every call, so a
value or an envelope is seen as it arrives (`live.call`).

### F-65 · gap (observed) · §5.2: `app`, and what `detail` is, in a JSON envelope

**Status at 0.8: resolved by 0.7.** §5.2: any operation may refuse with `app`; with no `error` type there is no detail, with one it is optional, and a raw type's is base64 text in a JSON envelope. zk2py's owner sends `app` without a detail, as the rule says. The Rust owner example has sent no detail on `@op/refuse` since FH2 (#660), so the runner's known deviation is now a plain check, which passes.

> "`detail` | a value, bytes, or null | With `app` only: the operation's
> declared `error` type, as a value inline (JSON, CBOR) or as its encoded
> message (protobuf)"

Three cases are not covered:
- **An operation that declares no `error` type.** May it reply `app`,
  and is `detail` then null?
- **A raw `error` type.** The envelope is JSON ("A raw type: the envelope is
  JSON"), and the detail is bytes. Is that base64 text, §7.2's JSON form of
  bytes?
- **A JSON Schema operation whose `error` type is raw or absent.** The same
  question.

Observed on the reference owner example, whose documented contract
refuses "any other types … with an `app` error envelope":
- every protobuf operation got `app` in `zk2.core.v1.Error`, with an empty
  `detail`;
- every JSON-enveloped operation got `{"code": "internal", "message": "the
  app detail does not fit the envelope's encoding"}` instead. That held with
  no `error` type, with `error = "json:Status"`, with `error = { raw =
  "text/plain" }`, and with `encoding = "cbor"` (as CBOR).

So the reference has no `app` envelope for those operations, and the spec
does not say whether that is right.
**Resolved:** zk2py's owner sends `app` without a detail, and `invalid_request`
for a malformed JSON request. zk2py's runner accepts any envelope that
decodes for the Rust owner's JSON operation, and records the code.

### F-66 · ambiguity · §4.3 minting: "plus one tick"

**Status at 0.7: resolved by 0.7.** §4.3: a tick is the timestamp type's smallest step, one NTP64 unit, and any larger step, zenoh-python's 1 ns included, keeps S7. zk2py's guess was allowed.

> "An owner therefore mints each state timestamp as the greater of
> `Session::new_timestamp()` and the last timestamp it issued plus one
> tick, with its session's zid as the id."

The spec does not define a tick. NTP64's own unit is 2^−32 s (about 0.23
ns), an HLC's is whatever it increments by, and zenoh-python builds an
`NTP64` from seconds and nanoseconds only. So a Python owner's smallest step
is 1 ns, about four of NTP64's units.
**Resolved:** a guess. zk2py adds 1 ns (`Owner.mint`). Any positive step
keeps S7's "never at or below the last", which is what the rule protects.

### F-67 · ambiguity · §3.3 and §8.2: the descriptor's first put

**Status at 0.7: resolved by 0.7.** §3.3, §8.2 step 3: the first descriptor is put when its queryable is declared, before the contract queryables and any token. zk2py now puts it there; it used to come after the contract queryables.

> §3.3: "The owner MUST put the descriptor on its instance key whenever it
> changes, and MUST answer a GET there with the current one."

Two points are open:
- **The first descriptor.** Is an owner's first descriptor a "change" that
  must be put?
- **Its place in the order.** §8.2's order names the descriptor's
  *queryable* (step 3), not a put. A subscriber to instance keys, as
  presence.md §1's tool is, sees a put only if there is one, and
  a late one could land after the tokens.

**Resolved:** a guess. zk2py's owner puts the descriptor once at start,
stamped, after step 3 and before the tokens.

### F-68 · gap · §8.2 and §4.2: the first state value against the tokens

**Status at 0.8: resolved by 0.7.** §8.2 "State values": a value held at start SHOULD be put before step 4. zk2py's guess was the rule. The Rust owner example has put its value before starting since FH2 (#660), so the runner's first-sight GET is now a plain check, which passes.

§8.2's order makes "alive ⇒ callable" hold for operations: queryables come
before tokens. A state resource's first *value* has no place in that order.
An owner that declares its publisher and state queryable (step 1), then its
tokens (step 4), and only then puts its first value, answers a GET made the
moment its interface token appears with silence. S6 says silence is not a
verdict, but a consumer acting on presence then reads nothing for a state
the owner was about to set.

**Resolved:** a guess. zk2py's owner puts every state value before its
tokens. The Rust owner example's value was present whenever zk2py read it.
That was after presence and the descriptor GET, though, so the runner does
not test the moment the token appears.

### F-69 · gap · §4.2 S1 and `state.md` §1: an owner stamp that cannot be told from a router's

**Status at 0.8: resolved by 0.7.** §4.2 "Observing S1" and state.md §1: the owner and the consumer are clients of a router with timestamping on, against an unstamped control put. zk2py's owner follows it in full (`run_python_s1`). The Rust owner example now takes `--connect`, so `run_rust_behind_r1` runs it as a client of R1: the value it holds from the start carries its own session's zid (its descriptor's `meta.zid`), not R1's. It publishes no data, so the scenario's other steps stay with zk2py's owner.

> state.md §1: "Every sample, the delete included, carries a timestamp
> whose id is the owner session's zid. A router stamp would carry the
> router's."

Both the Rust owner example and zk2py's owner are their own routers in the
runner. There, the owner session's zid is the router's, so the check
cannot tell an owner stamp from a router stamp. The runner's S1 check
passes in both cases, but it proves only that the stamp's id is the
process's.

This is F-61's situation again, for state: the scenario needs the owner as
a client of a separate router, which state.md does not say.
**Resolved:** zk2py's owner can run as a client of a router
(`Owner(connect=…)`), which `run_python_refusal` uses. The S1 check against
an owner behind a separate router is left for when the Rust owner can be
one too.

### F-70 · gap · §8.2 step 2: what "exposed" means at start-up

**Status at 0.7: resolved by 0.7.** §8.2 "Exposed": what serves a resource is declared; a state's value is not needed, and a template with no member is exposed by its template. zk2py's guess (a value needed; a required template refused) was overturned. zk2py now exposes every resource it is not told to withhold: states by their queryables and publisher (a raw one holds `ok`, others no value), streams by their publisher, events and templates by nothing more, and operations by a queryable on the key or over the template. Step 2 refuses what the rule refuses.

> §8.2: "2. validate that every required resource is exposed"; §2.3: "An
> owner MUST expose every required one, or not start (§8.2)."

"Exposed" is defined for the descriptor ("The exposed resources of an
interface are its contract's resources, minus …", §3.3), not for the
runtime check. The spec does not say what being exposed takes:
- for a state resource: a publisher, a queryable, a value;
- for a templated resource with no member yet: a queryable over its
  template.

An owner that serves only some resources cannot know whether the others
count as exposed.
**Resolved:** a guess. zk2py's minimal owner counts a parameterless raw
state, once its publisher, queryable and value exist, and a parameterless
operation, once its queryable exists. It serves nothing templated, so it
refuses to start for a contract with a required templated resource.

## New at 0.7 (#609)

Found while following 0.7: `just py-conformance` passes 514 of 514.
`just py-live` passes 129 of 129, with 2 known deviations of the Rust owner
example.

### F-71 · ambiguity · §7.3 (0.7): what "the null schema" is

**Status at 0.8: resolved by 0.8.** §7.3: "Exactly `null`" reads the `type` as a set of names, so `"null"` and `["null"]` are both the null schema (`compat/payload/jsonschema/nullable-null-as-list`). zk2py's guess (the string only) was overturned, and `compat._is_null_schema` now reads the set.

> "holding two branches in either order: the null schema, whose `type` is
> exactly `null` and which holds nothing else that carries meaning, and any
> schema S"

`type` may be a string or a list (§7.3 keeps `type` as JSON Schema has it).
So `{"type": ["null"]}` admits exactly null too, but is it "exactly
`null`"? No fixture has one. A generator writing the list form would make
the same `Option` a nullable form for one implementation and a plain
`anyOf` for another, whose classes then differ (compatible or review).
**Resolved:** a guess. zk2py takes only the string `"null"`
(`compat._is_null_schema`).

### F-72 · ambiguity · §9.8 (0.7): a recursive `$ref` "compared as written"

**Status at 0.8: resolved by 0.8.** §9.8: "as written" means by its text, the `$ref` value and its siblings with annotations dropped. So renaming a recursive definition reached from inside `oneOf`/`anyOf`/`prefixItems` is review (`anyof-recursive-renamed`), and an annotation added there is no change (`anyof-recursive-described`). zk2py's guess (by target) was overturned, and `compat.written` now keeps the back-reference as it was written. Under the guess, zk2py failed `anyof-recursive-described`, and passed `anyof-recursive-renamed` only by accident: the wrapped revisions' artifacts have different stems (`old`, `new`), so even the normalized target text differed.

> "A `$ref` back to a target already being followed is compared as written,
> which ends a recursive type"

"As written" could mean the `$ref`'s text, or the target it names. Two
revisions can spell one target two ways:
- `#/$defs/Node` in the defining file;
- `nodes.json#/$defs/Node` from another file.

By text they differ, so the change is review. By target nothing changed.
No fixture has a recursive type inside `oneOf`/`anyOf`/`prefixItems`.
**Resolved:** a guess. zk2py compares such a `$ref` by its target, written
as `<stem>.json#<pointer>`, so a respelling is no change
(`compat.written`).

### F-73 · gap · `state.md` §1 step 3 (0.7): a precondition a tester cannot arrange

**Status at 0.8: resolved by 0.8.** state.md §1 step 3 now asks only that v4's stamp exceed v3's. The tick is checked "with a clock the implementation controls". zk2py's runner checks the order. It then sets `Owner.clock` 5 s behind the last stamp, and checks that the next stamp is that stamp plus 1 ns, zk2py's tick.

> "3. The owner puts v3 and v4 back to back, faster than its clock
> advances. … v4's timestamp is greater than v3's, by at least one tick"

Whether two puts land within one clock reading is the owner's timing,
which a black-box tester cannot force. zk2py's owner, put through
zenoh-python, issued v3 and v4 53 µs apart. The HLC had advanced, so the
"plus one tick" branch of §4.3's minting never ran, and the check passed
without testing it. The scenario does not say how a run shows the branch
was taken, nor whether a run that did not take it counts.
**Resolved:** the runner checks what the scenario expects (v4 > v3 by at
least a tick). It reports the gap in nanoseconds, which shows the branch
was not exercised. zk2py's minting branch is exercised only by its own
logic, not by a test here.

## New at 0.8 (#609)

Found while following 0.8. `just py-conformance` passes 517 of 517.
`just py-live` passes 163 of 163, with 2 known deviations of the Rust owner
example, both from one cause (below). zk2py now runs, against its own
owners, the scenarios 0.8 rewrote or added:
- presence.md §6, both halves, on a zenoh-python router with the access
  control the scenario names. A refused read and a stalled one behave as
  0.8 measured, and so does the `egress` control.
- operations.md §1, by behaviour. A call returns on its first reply while
  its server holds the query open 0.5 s, and the same call under `Latest`
  waits the 0.5 s. Two instances on one router run 200 calls between them
  (200 and 0), and run 2 under target `All`. Across two routers they run
  200 each.
- operations.md §2 steps 1 to 5, and §3 step 3.

**Observed on the reference, not a finding.** The owner example declares
no queryable over an operation template. With `zk2py_tc.v1`, it starts,
holds the interface token, and lists nothing `unavailable`. Yet every call
to `interfaces/{if}/set` or `…/reset` is silent, whether concrete,
malformed (`ETH0`) or a fan-out, while `diagnostics` answers. O1 wants the
queryable "on its concrete key (or its template)", and §8.2 "Exposed" says
a template is exposed by its template. The runner reports both calls as
known deviations (`run_rust_behind_r1`). This also leaves F-74 and F-76
unobservable on the reference.
**Fixed with 0.9:** the owner example serves every operation now, a
templated one over the whole template. The two known deviations are plain
checks, and both pass.

### F-74 · ambiguity · §5.1 "Over a template" (0.8) against O2: two refusals for one call

**Status at 0.9: resolved by 0.9.** §5.1 "The order of refusals": before any handler, `fanout_forbidden` (O2), then `unavailable` (O3), then a key that names no member. zk2py's guess was the rule. operations.md §2 step 1 now makes the call, and zk2py's owners and the owner example both answer `fanout_forbidden`. zk2py's `unavailable` queryable answered every call `unavailable`, wildcards included, so it now checks O2 first too. The runner pins the whole order on a gated operation no host exposes.

> O2: "a call whose key expression is not concrete MUST be refused with
> `fanout_forbidden`, unless the operation declares `fanout = "allowed"`"

> §5.1 (0.8): "A concrete parameter chunk that is not a canonical slug
> (§1.4) names no member. A server over the template refuses such a call
> `invalid_request` before any handler runs, fan-out or not"

`zk2/*/tc/tc.v1/@op/interfaces/ETH0/set` is both:
- not concrete, on an operation that forbids fan-out;
- holding a concrete parameter chunk that is not canonical.

Each rule refuses it before any handler, with a different code, and the
call gets one envelope. No step of operations.md §2 combines the two:
- step 1's `*` sits at the parameter, so no chunk is malformed;
- step 4's `ETH0` targets `reset`, which allows fan-out.
**Resolved:** a guess. zk2py checks O2 first, on the key expression as a
whole, before it reads the template, so the call is `fanout_forbidden`
(`owner._op_handler`).

### F-75 · gap · presence.md §6 step 3 (0.8): a read that selects no token

**Status at 0.9: resolved by 0.9.** presence.md §6 step 3 now reads `zk2/*/*/@zk/instance/*` with an owner present: the open read holds the token, and the held one is empty with `Timeout`. That was zk2py's fix, and the runner now runs the step as written, without the `zk2/**` pair.

> "3. The second tool reads `zk2/**` with the link flowing, then again with
> R1's replies held back past the read's timeout." Expected: "The second
> ends with the error reply `Timeout` and no token"

§1.3's guard: an ambient selector such as `zk2/**` "never reaches … a
control key", because `**` never matches a verbatim chunk. So `zk2/**`
returns no liveliness token even with the link flowing (measured: complete,
no token, no error reply). The stalled read's "no token" therefore cannot
fail, and the step does not show that the stall hid anything. The
`Timeout` reply alone carries the evidence.
**Resolved:** zk2py runs the step as written, and the same pair on
`zk2/*/*/@zk/instance/*`. That read holds the owner's token while the link
flows, has none and the `Timeout` reply while it is stalled, and has the
token again once the link is released.

### F-76 · ambiguity · §5.1 (0.8): a `replies = "many"` handler that names no member

**Status at 0.9: resolved by 0.9.** §5.1 "Sending nothing needs no member": with no `summary`, zero values then completion; with one, `internal`. zk2py's guess was the rule. operations.md §2 step 6 pins it, and zk2py's owner passes it: no value, no envelope, and the handler ran. The owner example's handler always echoes, so step 6 does not apply to it. Its unbound fan-out is `internal`, as §5.1 says of a server that would reply but names no member.

> "It names the member each reply answers for, and replies on that
> member's key … One that names no member has no key to reply on, and
> refuses the call: the reference refuses it `internal`."

> "With `replies = "many"`, a declared `summary` is owed too … Without one,
> zero values then completion is the operation's own answer"

0.8 makes the template rule hold "whatever `replies` is". Take a
template-wide `many` operation with no `summary`, called over a wildcard,
whose handler finds nothing to report: it names no member and sends no
value. By the first rule it refuses (`internal`). By the second, zero
values then completion is its answer. A caller sees an envelope in one
reading and silence in the other.
**Resolved:** a guess. zk2py's owner reads it by the second rule: a `many`
handler that sends nothing ends with completion alone, unless a summary
is owed or the handler raised (`OpCall.finish`). `internal` is reserved for
a call that needed an answer, and for zk2py's default handler, which names
no member for a fan-out and says so.

## At 0.9 (#609)

`just py-conformance` passes 517 of 517: 0.9 adds no fixture. `just
py-live` passes 170 of 170, with no known deviation.

What changed in zk2py:
- **The refusal order.** The owner's `unavailable` queryable now checks O2
  first. It was the one place where zk2py's order differed from 0.9's.
- **operations.md §2** gains step 1's `ETH0/set` call and step 6. A
  further check pins the whole order on `calibrate`, an optional operation
  gated on a capability no host holds:
  - a wildcard call is `fanout_forbidden` from each host;
  - a concrete call with `ETH0` is `unavailable`, with cause `capability`,
    not `invalid_request`.
- **presence.md §6 step 3** reads the instance tokens, as amended.
- **The owner example's templated operations** are plain checks now. Behind
  R1 it also answers:
  - step 1's `ETH0/set` with `fanout_forbidden`;
  - step 4's `ETH0/reset` with `invalid_request`;
  - a fan-out that leaves the parameter unbound with `internal`.

Implementing 0.9 raised no new question.

## New at 0.10 (#609)

Found while following 0.10. `just py-conformance` passes 518 of 518, with
`descriptors/ok-optional-role` read through zk2py's strict shape: the
schema is read from `spec/` at run time. `just py-live` passes 190 of 190,
with no known deviation.

What zk2py does with 0.10:
- **Its owner** writes `optional: true` for an optional role, and nothing
  for a required one. It already stated `meta.zid`.
- **Both owners' descriptors are checked for 0.10's additions.**
  - The owner example's descriptor states `meta.zid`. Behind R1, the stamp
    of the value it holds from the start is attributed to it by that zid.
  - Serving `zs.thresholds.v1`, the owner example writes
    `optional: true` for the optional role `desired`.
  - zk2py's `zk2py_needs.v1` control writes it for `peer` and not for
    `upstream`.
- **S1 attribution.** A tool attributes a stamp by the descriptor it reads
  (`live.attribute_stamp`). Through R1, zk2py's owner's three samples read
  `owner`, and the router-stamped control reads `foreign`.
- **S4 through the admin space** (`live.check_s4`).
  - With R1's admin space off, zenoh 1.10.1's default, nothing answers
    `@/*/router`, and the check is unobservable.
  - Enabled read-only, R1 answers with `"plugins": null`, so it runs no
    storage, and the check is clean.
- **Presence shapes in two reads** (`live.presence_faults`, a 1 s grace).
  zk2py adds an interface token that the owner's descriptor does not list,
  under the owner's instance.
  - Removed within the grace, it is seen once and passes.
  - Kept, it is seen in both reads, and it is a fault.
- **Revisions on the bus** (`compat.classify_pair`). Two zk2py owners serve
  `zk2py_bringup.v1` at minor 0 and at minor 1, which adds an optional
  operation.
  - Ordered by the minors their descriptors state, the pair is
    compatible.
  - Classified both ways, it is undecided: the removal is breaking.
- **Not covered:**
  - a token count as a budget's lower bound, since zk2py counts no budget;
  - an archive's alignment, since zk2py has no archive.

**Editorial, not a finding.** In §9.8, the new bullet "On the bus,
revisions carry no order" ends "… and undecided when they disagree. Each
rule below is a transition from the earlier revision to the candidate, and
its class already accounts for …". The paragraph that introduces the rules
was swallowed into the bullet. The meaning survives. **Fixed in 0.11:**
the paragraph is whole again, and the bullet follows it.

### F-77 · gap · §3.3 (0.10): an `optional` that does not repeat its contract

**Status at 0.11: resolved by 0.11.** §3.3: for a contract's role a tool takes the need from the contract, and a disagreeing `optional` is listed under "Not checked, deliberately". `false` written out is the same as absent. `descriptors/ok-optional-unchecked` pins both with no code, and zk2py passes it. zk2py's guess was the rule.

> "`optional` (0.10) is `true` for a role the instance works without, and
> absent otherwise: absent is required. For a role a contract declares, it
> repeats that contract's `[requires]`."

Two descriptors break that sentence, and the checks do not say what to
report for either:
- an entry `declared_by` a given contract whose `optional` differs from
  that contract's `[requires]`, for instance a required role marked
  `optional: true`;
- an entry with `"optional": false`. The schema admits it, but the prose
  says "absent otherwise".

The D codes name no such condition. "Not checked, deliberately" does not
list it either, though that list exists to make unchecked conditions
explicit (F-06). Its nearest item is "that a role `declared_by` an
interface is in that contract's `[requires]`", which suggests the role's
agreement with its contract is unchecked as a whole. No fixture has either
case.
**Resolved:** a guess. zk2py's checker reports no code for either (both
measured: `[]`), reading the unchecked item as covering `optional` too.
zk2py's owner writes neither. Its runner checks both owners' descriptors
against the rule as stated.

### F-78 · gap · §3.3 `meta.zid` (0.10): a zid has no spelling

**Status at 0.11: resolved by 0.11.** §3.3 "How a zid compares": a tool MUST compare two zids by value, never by text. An owner SHOULD write `meta.zid` as zenoh writes it, which Appendix B now records: lowercase hex without leading zeros. The reference's doctor compared text, a bug fixed with 0.11. zk2py already compared by value. It now treats a `meta.zid` that is not hex as stating no zid (unattributable), rather than comparing it as text. The runner checks that both owners write the zenoh form, and that the reference's real zid, respelled in capitals and padded to 32 digits, still attributes its stamp to it.

> "an owner SHOULD state its session's zid as `meta.zid` … A tool
> attributes a state stamp to its owner by comparing the stamp's id with
> it"

The core gives an instance id a form ("16 lowercase hex digits", D002), but
a zid none. The fixture `ok-optional-role` writes 16 hex digits. zenoh
1.10.1 writes a zid as hex without leading zeros: the owner example's
`meta.zid` was `f385f26aeb53faaf5066e9b5be2aef8`, 31 digits, and so was its
stamps' id in zenoh-python. Both sides came from one zenoh, so the texts
matched. An owner or a tool on another binding that pads to 32 digits, or
writes uppercase, would make a textual comparison call the owner's own
stamp `foreign`, which 0.10's "never foreign" wording is there to prevent.
**Resolved:** a guess. zk2py compares two hex spellings as numbers
(`live.attribute_stamp`), so padding and case do not matter, and any other
spelling as text.

### F-79 · gap · §4.2 S4 (0.10): what "the routers' storage admin space" is

**Status at 0.11: resolved by 0.11.** §4.2 "What the check reads": `@/*/router` for the routers that answer, and `@/*/router/**/storage_manager/storages/**`, one key per storage with its `key_expr`. A storage intersecting an owner's `state/**` or `@state/**` breaks S4. A router answering the first with nothing under the second runs no storage, and with no router answering the check is unobservable. zk2py now reads both selectors (`live.check_s4`) and keeps `plugins` beside them; the two agree on R1. A stand-in admin record shows the storages read: a storage on `telemetry/**` is clean, and one on `zk2/**` breaks S4. A gap remains in who may answer the first selector: F-80.

> "A **tool** checks S4 against the routers' storage admin space … The
> admin space is off by default … Without it, a tool reports the check
> unobservable, never clean."

The spec says when the check is unobservable, but not how it is made:
- which admin keys list a router's storages;
- how a storage's key expression is read there;
- what shows that a router runs no storage at all.

Appendix B records only that the admin space is off by default. An
independent tool learns the rest from zenoh, not from the spec. Measured
on zenoh 1.10.1:
- a zenoh-python router answers `@/<zid>/router` only once
  `adminspace.enabled` is true;
- its record carries `"plugins": null`;
- `@/*/router/status/plugins/**` is then empty.
**Resolved:** zk2py reads `@/*/router`. No answer makes the check
unobservable. A record with no plugin is clean, since there is no storage
manager to run a storage. Any plugin is unobservable too, because zk2py
does not guess the storage manager's keys. The storages branch could not be
built from the spec, nor run here: zenoh-python hosts no plugin.

## New at 0.11 (#609)

Found while following 0.11. `just py-conformance` passes 519 of 519, with
`descriptors/ok-optional-unchecked`. `just py-live` passes 194 of 194, with
no known deviation.

What changed in zk2py:
- **S4** reads 0.11's two selectors. `plugins` is kept beside them, and
  the two agree on a router with no plugin. With no router to run a real
  storage on, a stand-in admin record (queryables the runner declares)
  shows the storages read:
  - a storage on `telemetry/**` leaves S4 clean;
  - one on `zk2/**` intersects owners' `state/**`, and breaks it.
- **zids compare by value only.** A non-hex `meta.zid` is unattributable.
  The runner checks that both owners write `meta.zid` as zenoh writes it,
  and that a respelling of the reference's zid still attributes its stamp
  to it.

### F-80 · gap · §4.2 S4 (0.11): who answers `@/*/router`

**Status at 0.12: resolved by 0.12, against zk2py's fix.** §4.2 "Who answered": an admin answer counts as a router's only when the reply's replier id is the zid its key names, and that zid is a router the session is connected to, or the session itself. Any other answer is unverified and never contributes to a clean verdict. A tool that cannot read the replier id holds every answer unverified. An operator MAY tell a tool to trust every answer. zk2py's 0.11 defence judged by the key's zid, which the reference showed a spoofer defeats by answering on the real router's own key. zk2py reproduced this: with R1's admin space off, a client's answer on `@/<R1>/router` is the only answer, and its replier id is the client's. zenoh-python 1.10.1 exposes `Reply.replier_id` (an `EntityGlobalId`, whose `zid` is the replying session's). Its stub marks it `@_unstable`, but the published wheel has it at run time. zk2py now verifies with it (`live.unverified_why`), and security.md §3 steps 1–2 pass.

> "`@/*/router`, the routers that answer … A router that answers the first
> selector and has nothing under the second runs no storage. When no router
> answers the first, the check is unobservable."

A queryable on `@/<id>/router` is not a router's alone: any session can
declare one. Measured on zenoh 1.10.1, with R1's admin space off (the
default):
- the check is unobservable, as it should be;
- once a client session of R1 declares a queryable on
  `@/fedcba9876543210/router` answering `{"plugins": null}`, that answer
  is "a router that answers the first selector and has nothing under the
  second". The check then reads clean.

The same session could make a real storage disappear from the reading
only by hiding the real router's answer, which it cannot do. It can,
though, turn "unobservable", the one verdict 0.10 guards ("never clean"),
into "clean". The spec says nothing about telling a router's record from
a session's. The grants (§11.1) give no principal declarations under `@/`,
but under `allow`, the default the core assumes elsewhere, nothing stops
one. A tool's own grants do not help either.
**Resolved:** zk2py takes an answer for a router's only when its id is one
of the routers its own session is connected to (`session.info.routers_zid()`,
compared by value). With none of those answering, the check stays
unobservable, and the other ids are reported as unverified. Storages
listed under an unverified id still count, since they can only make the
verdict worse. A router further away is then unverified too, which costs a
clean verdict, never a false one.

## New at 0.12 (#609)

Found while following 0.12. `just py-conformance` passes 519 of 519 (0.12
adds no fixture). `just py-live` passes 197 of 197, with no known
deviation.

**Does zenoh-python 1.10.1 expose a reply's replier id? Yes.**
- `zenoh.Reply` has `replier_id`, next to `ok`, `err` and `result`.
- Its type stub declares `@property @_unstable def replier_id(self) ->
  EntityGlobalId | None`, documented "the ID of the zenoh instance that
  answered this reply".
- `_unstable` is a marker in the stub only: the published abi3 wheel has
  the attribute at run time.
- `EntityGlobalId.zid` is the replying session's `ZenohId`, and `eid` its
  entity: 1 for a router's own admin answer, 6 for a client's queryable.

zk2py reads it (`live.replier_of`). A binding without it would yield None,
and every admin answer would then be held unverified.

What changed in zk2py:
- **Who answered.** Every answer to S4's two selectors is verified by its
  replier id: it must be the key's zid, and that zid a router of the
  session, or the session itself (`live.unverified_why`).
  - Unverified answers are unjudged: they never break S4, and never let
    it be clean.
  - Each is listed with its key, its replier, and why: no replier id; the
    replier is not the key's zid; or not a router of this session.
  - `check_s4(trust=True)` is the operator's alternative.
  - The 0.11 defence, judging by the key's zid, is gone. On its own it
    would have read security.md §3 step 1 as clean.
- **security.md §3 steps 1 and 2**, as measured:
  - **Admin space off.** A client `S` answers on `@/<R1>/router`, and its
    answer is the only one. Its replier id is S's zid. It is unverified,
    and the check is unobservable.
  - **Admin space on, read-only.** R1 answers under its own replier id,
    verified, and S's answer is still unverified. The check is
    unobservable, not clean.
  - **Step 3** waits for the grant generator, as the scenario says.
- **The storage stand-in** is a client playing a storage manager, which is
  the spoof. Untrusted, its answers are unverified, and even a storage on
  `zk2/**` leaves the check unobservable. Trusted, as the reference's own
  test now runs, `telemetry/**` is clean and `zk2/**` breaks S4.

### F-81 · gap · §4.2 "Who answered" (0.12): a client tool verifies one router

**Status at 0.13: resolved by 0.13.** §4.2 "Verified routers, outward": the routers the session is connected to, and the session itself, are verified, and so is every zid a verified router's own answer lists among its `sessions` with `whatami` `router`, until nothing new is verified. Appendix B states the document's shape. zk2py implements it (`live.verified_routers`) and gives each unverified answer the reference doctor's reason. A client tool on R1 now verifies R2 through R1's document, and S4 reads clean (security.md §3 step 4). A client answering on its own key stays unverified, since R1 lists it as `client`. The peer tool, kept as a cross-check, reads the same verdict directly.

> "A tool counts an answer as a router's only when the reply's replier id
> is the zid the key names, and that zid is a router its session is
> connected to, or the session itself. Any other answer is unverified, and
> an unverified answer never contributes to a clean verdict."

> Appendix B: "A client connects to one endpoint at a time."

A router further away answers honestly: its replier id is the zid its key
names. But it is not a router the tool's session is connected to, so its
answer is unverified, and the check can never be clean beside it. Measured
on zenoh 1.10.1, with R2 linked to R1 and both admin spaces on, read-only:
- **A client tool on R1.** It reads R1's answer, verified, and R2's,
  carrying R2's own replier id but unverified ("not a router of this
  session"). The check is unobservable.
- **A peer tool connected to both routers.** `info.routers_zid()` lists
  both, both answers are verified, and the check is clean. The peer
  receives R2's record twice, by two paths, which zk2py counts once.

So in any deployment with more than one router, a tool connected as a
client, the usual shape, can never report S4 clean. It needs a peer
session connected to every router, one session per router, or the
operator's trust. The spec does not say which, nor whether a far router's
self-consistent answer is meant to count. Requiring the connection may be
deliberate, since a transport's peer id is what a session can vouch for.
The cost is not stated.
**Resolved:** zk2py follows the rule as written. It reports a far router's
answer as unverified with its own reason ("not a router of this session"),
distinct from a spoof ("replier is not the key's zid"). The runner shows
that a peer tool connected to every router gets the clean verdict.

## At 0.13 (#609)

`just py-conformance` passes 519 of 519 (0.13 adds no fixture). `just
py-live` passes 198 of 198, with no known deviation.

What changed in zk2py:
- **Verified routers, outward** (`live.verified_routers`). The base is
  the session's routers (`info.routers_zid()`) and the session itself. Each
  verified router's self-consistent answer adds the `peer` of every session
  it lists with `whatami` `router`, until nothing new is added.
- **Each answer is judged against that set** (`live.unverified_why`), with
  the reference doctor's reasons:
  - no replier id;
  - a replier other than its key's router;
  - no verified router lists it (a router record);
  - its router's own answer unverified (a storage record).
- **security.md §3 step 4.** R2 links to R1, both admin spaces on,
  read-only, and the tool is a client of R1.
  - R1's document lists R2 as `router`, and the tool, and `S`, as
    `client`.
  - R2's answer carries R2's own replier id, so both routers are verified,
    and the check is clean.
  - `S` then answers `@/<S's zid>/router` under its own replier id. It stays
    unverified, "no verified router lists it", and the check is not clean.
- **The peer tool** connected to both routers is kept as a cross-check: it
  verifies both directly, and reads the same verdict.

Implementing 0.13 raised no new question.

## New at 0.14: access control from §11 (#609)

This round tested whether §11 is enough to build access control from.
zk2py gained a grant generator, `zk2py.acl`, written from §11.1–§11.2
alone, and a live run, `zk2py.acl_interop` (`--only acl`).
- The run has a deployment of its own:
  - two owners, `h1/tc` and `h2/tc`, each serving `zk2py_echo.v1`,
    `zk2py_tc.v1` and `zk2py_bringup.v1`;
  - a consumer, a caller, the same caller with its presence removed, and
    a Tool;
  - a client `S`, authenticated by R1 but no principal of the deployment.
- Each principal is bound by a usrpwd user (§11.3).
- The generator compiles the deployment into zenoh's `access_control`
  block: 48 allow rules under `deny`, 59 deny rules under `allow`. Each
  posture and each generator-check variant runs on a zenoh-python router of
  its own.

`just py-conformance` passes 519 of 519 (0.14 adds no fixture). `just
py-live` passes 223 of 223, with no known deviation, 25 of them in the
access-control run.

**What the run shows, against security.md and 0.14:**
- **§1 under `deny`.**
  - An owner's put reaches the consumer that names it.
  - A subscription the bindings do not name is blocked.
  - The fan-in GET over `zk2/*/tc/…` gets one reply per backend.
  - A put, a queryable and a token on another principal's keys are all
    blocked, and so is a put on a wildcard key.
  - The caller reads h1's instance and interface tokens. With its
    presence removed, the same read is complete and empty.
  - A contract fetch works for every principal.
- **0.14, presence reads the descriptor.** The caller's descriptor GET is
  answered, and its descriptor subscription receives both owners' first
  puts. Without presence, both are silent.
- **§2 under `allow`.**
  - The same forgeries and the unnamed subscription are blocked, except a
    put on a wildcard key, which reaches the consumer.
  - The deny posture's allow rules, under `default_permission: allow`,
    block nothing.
- **§2's generator check, and 0.14's reply rule.** Under `deny`:
  - Without the consumer selectors in the providers' egress, the fan-in GET
    gets 0 replies. A concrete GET still gets its one.
  - Without them in the providers' ingress `reply`, a wildcard
    `diagnostics` call still gets both values: a value reply is checked
    against its own key.
  - A wildcard `set` call reaches both providers, both refuse it, and no
    refusal reaches the caller: a refusal is checked against the query's
    key.
- **0.14 under `allow`.**
  - The caller's ungranted wildcard GET gets nothing back.
  - The consumer's wildcard call to `diagnostics`, `fanout = "allowed"` and
    never granted to it, executes on both providers, and only its answers
    are denied.
- **§11.2.** A binding narrowed to one member (`tracks/t1`) makes the
  generator warn `complement_partial`.
- **§11.3.** With usrpwd on R1, a session with no credentials or a wrong
  password is refused at the link. `S`, authenticated but matching no
  subject, reads everything under `allow`.
- **security.md §3 step 3.** The generated grants refuse a principal's
  queryable (`own-h1`) on `@/<R1>/router` under both postures. They refuse
  `S`'s under `deny` only (F-88).

**§11 rules zk2py could not build from the text alone:**
- **The deployment input** (F-82): invented.
- **The messages and flows** each shape compiles to (F-83): read from the
  CHANGELOG's account of the reference generator, and measured.
- **The admin-space read for a tool** (F-84): added.
- **The complement's universe under `allow`** (F-85): guessed.
- **"Every principal" under `allow`** (F-87): measured, and compiled per
  policy.
- **Not built at all, because nothing here exercises them:**
  - §8.5's face declarations toward a far router in a south region, which
    need a regional deployment;
  - the History, Archive and union-storage grants;
  - the namespace, which this deployment does not use.

### F-82 · gap · §11 (0.14): no deployment to generate from

**Status at 0.15: resolved by 0.15.** §11.1 "The input" states what a generator compiles: principals bound to a user name or CN, the services each runs with their contracts, bindings, calls, inspected services and the admin read, archives, and the namespace. "The format is the generator's own"; the reference's enrollment is informative. zk2py keeps `acl.Principal` and `acl.Use`, and takes neither history nor archives nor a namespace, which its deployments do not use.

> security.md: "Grants are generated from contracts and bindings."
> §11.1: "Consume | Subscribe or GET on the prefixes a principal's bindings
> name … Call | Query on specific …/@op/<op> keys … Tool | … on what it
> reads and calls, and presence on the services it inspects."

A generator needs, per principal:
- the service it owns;
- its bindings;
- the operations it calls;
- the services it inspects;
- the username or certificate CN that binds it (§11.3).

The spec defines none of them as an input. R1 makes bindings a matter of
configuration, and the only bindings file in `examples/zk2/` calls its
shape "recommended, not normative". Nothing names what a caller calls, or
what a tool inspects, in any file.
**Resolved:** zk2py's own input, `acl.Principal` and `acl.Use`. A
principal has a username, an optional service and the contracts it
implements, uses for Consume and for Call, and services it inspects. A
`Use` has an interface, provider patterns, and resources named
`<kind token>/<template>`.

### F-83 · gap (measured) · §11.1 (0.14): actions, not messages and flows

**Status at 0.15: resolved by 0.15.** §11.2 "Messages and flows" is a table per grant, which both implementations measured the same way, the liveliness read's egress `liveliness_token` included. zk2py's pairs were already the table's. Its docstring now cites the table.

> "Own | A service principal puts, deletes, declares queryables, replies
> and declares tokens … On egress, it receives queries and subscriber
> declarations"; "presence: liveliness reads (GETs and subscriptions) …
> and the descriptor's GET and subscription"

zenoh 1.10.1's access control has nine messages and two flows, and a rule
grants pairs of them. §11.1 states actions, and leaves the pairs to the
implementer, Own's egress aside.
- **The CHANGELOG states some pairs, not the core.** It says of the
  reference generator that presence "gain[s] query and declare_subscriber
  on ingress, and reply and put on egress".
- **The liveliness pair could only be found by measuring.** Under `deny`,
  a liveliness GET needs `liveliness_query` on ingress and
  `liveliness_token` on egress. `liveliness_query` on both flows, or with
  `reply` on egress, returns no token.
- **The reader's selector must also be included in the grant.** A
  wildcard read needs the wildcard granted, not the providers' concrete
  prefixes.
- **Consume and Call need their deliveries granted on egress** (`put`,
  `delete`, `reply`), which the table does not say either.

**Resolved:** zk2py's pairs, in `zk2py.acl`'s docstring. They are
measured where the run exercises them, and every check passes on them.

### F-84 · gap (measured) · §11.1 Tool and §4.2 S4 (0.14): reading the admin space

**Status at 0.15: resolved by 0.15.** §11.1 Tool: "A tool that checks S4 or runs a doctor also holds the admin read: `query` on `@/*/router` and `@/*/router/**`, and their `reply`, never namespaced." zk2py's grant was `@/**`, and is now those two keys. The run shows that under `deny` the tool reads `@/*/router` and runs S4, clean, while the caller, without the admin read, gets nothing.

> §4.2: "A deployment that wants S4 checked enables it, read-only, for the
> tools' principals"; §11.1: "No principal declares queryables under
> `@/**`"; "Tool | … Consume and Call on what it reads and calls, and
> presence on the services it inspects."

"Read-only, for the tools' principals" is the admin space's own setting.
Under `default_permission: deny`, the tool also needs an access-control
grant: `query` on ingress and `reply` on egress, on `@/**`. No shape
includes one, and §11.1 speaks of `@/**` only to forbid queryables there.
Measured: the caller, with no such grant, gets nothing from `@/*/router`.
The tool, granted it, reads R1's answer and runs S4. Under `allow`, every
principal reads the admin space, since no deny names it.
**Resolved:** zk2py's Tool shape takes an admin-space read
(`Principal.admin_read`), compiled to those two pairs.

### F-85 · ambiguity · §11.2 (0.14): the complement of a grant

**Status at 0.15: resolved by 0.15.** §11.2 "The complement's key set": the deployment's own keys, meaning declared resources, each service's `@zk/**` and the contract keys. Undeclared keys stay open under `allow`. This was zk2py's guess. The admin space is outside the set, so under `allow` every principal reads it. The run reports that as information.

> "Under `allow`, Zenoh does not evaluate allow rules. A grant compiles
> into denies of its complement, regenerated on every contract revision."

A key expression has no negation, and a deny works by inclusion, so a
complement has to be enumerated over some finite set of keys. §11.2 does
not say which set. "Regenerated on every contract revision" suggests the
contracts' keys, but the text gives no set and no granularity:
- **per service prefix:** too wide, since a deny of `zk2/h1/tc/**` would
  also deny a consumer's own concrete reads there;
- **per resource;**
- **per message and flow.**

Keys no contract declares are in no complement, so they stay open under
`allow`, which security.md §2's "every unauthorized action is blocked"
does not anticipate.
**Resolved:** a guess, `acl.universe`. The set is each served resource's
key expression, each service's `@zk/**` and the contract keys, taken for
each of the nine messages on each flow. A universe key that a grant
includes is left open. One a grant only intersects is the R2-narrowed case
(`complement_partial`, 0.14), and is left open too. Any other is denied.
The run's `allow` checks pass on it.

### F-86 · gap (measured) · §4.2 "What the check reads" (0.11): storage records that are not storages

**Status at 0.15: resolved by 0.15.** §4.2: "A storage is an answer to the second selector whose key the selector includes, ending …/storage_manager/storages/<name>. Other answers arrive too, and are not storages." zk2py's inclusion test was the rule. It now also requires the `storage_manager/storages/<name>` ending (`live.check_s4`).

> "`@/*/router/**/storage_manager/storages/**`, one key per storage …
> A router that answers the first selector and has nothing under the
> second runs no storage."

A router's admin space also holds `@/<zid>/router/queryable/<key expr>`,
one record per declared queryable. The key embeds the queryable's key
expression, wildcards included. A queryable over `…/state/**` (S2) gives a
record whose key ends in `**`, and that key intersects the storages
selector. Measured with two zk2py owners on R1, admin space on: the
storages selector is answered with six `router/queryable/…/state/**`
records, one per owner per interface, none a storage and none with a
`key_expr`. So with any owner present, "nothing under the second" never
holds. A tool that reads each answer as a storage finds none it can read,
and calls the check unobservable. zk2py's run showed exactly that before
the fix.
**Resolved:** zk2py keeps only an answer whose key the selector includes,
not merely intersects (`live.check_s4`). R1's real storage records would
be included, and the queryable records are not.

### F-87 · gap (measured) · §11.1 (0.12, 0.14): "every principal" under `allow`

**Status at 0.15: resolved by 0.15.** §11.2: "every principal" is a rule in each principal's own policy, never one catch-all subject's, with the measurement stated. zk2py already compiled it that way.

> "No principal declares queryables under `@/**` … A generator allows it
> to none under `deny`, and denies it to every principal under `allow`."

The natural compilation of "every principal" is one subject that matches
every session: no attribute, or `link_protocols: ["tcp"]`. Measured on
zenoh 1.10.1, with the per-user subjects of the generated `allow` block
beside such a subject:
- the per-user denies stop applying: `own-h1`'s put on h2's key arrives,
  and the caller's ungranted GET gets both replies;
- `own-h1`'s queryable on `@/<R1>/router` is answered;
- only `S`, which matches no per-user subject, is refused.

The spec does not say how to express "every principal", nor that this
compilation fails.
**Resolved:** the `@/**` deny goes into each principal's own policy. All
the run's `allow` checks pass with it, and `own-h1`'s spoof is refused.

### F-88 · contradiction · security.md §3 step 3 against §11.3 (0.14): a session that is no principal

**Status at 0.15: resolved by 0.15.** security.md §3 step 3 now runs `S` twice: as an enrolled principal, and as an authenticated session that is no principal. An enrolled principal is refused under both postures. A non-principal is refused under `deny`, but under `allow` its answer arrives, and only the replier id keeps S4 from clean. zk2py runs both: `S-enrolled` (user `spoofer`, holding only the open contract grants) and `S` (user `stranger`). It measures exactly those outcomes.

> security.md §3: "Under each posture, generate R1's grants … and repeat
> step 1." Expected: "R1 refuses `S`'s queryable."
> §11.3: "A session that matches no subject gets no policy, so it gets
> `default_permission`, which under `allow` is everything."

`S` is "a client `S` that is no router", and here no principal either.
Under `deny`, the generated grants refuse it: it matches no subject, so
it gets nothing. Under `allow`, it matches no subject, so no generated
deny reaches it:
- only a subject matching every session would, and that undoes the
  per-user denies (F-87);
- measured: `S`'s queryable on `@/<R1>/router` is answered under `allow`.

Step 3 holds under `allow` only for an `S` that is a principal, which the
run shows (`own-h1`'s spoof is refused), or for an `S` refused at the link,
which never declares anything. What still holds is 0.12's replier check:
`S`'s answer carries `S`'s replier id, stays unverified, and S4 is not
clean.
**Resolved:** zk2py checks step 3 for a principal under both postures, and
records `S` under `allow` as this finding, checking that S4 is not clean
beside it. A deployment under `allow` keeps every authenticated user a
principal of the deployment (§11.3's link MUST, extended to the
dictionary).

## At 0.15 (#609)

`just py-conformance` passes 519 of 519 (0.15 adds no fixture). `just
py-live` passes 223 of 223, with no known deviation. The access-control
run (`--only acl`) is 25 of them.

What changed in zk2py:
- **The generator follows §11.2's table** (`zk2py.acl`). Its pairs were
  already the table's. The admin read is now `@/*/router` and
  `@/*/router/**` rather than `@/**`. The complement's key set and the
  per-policy `@/**` deny were already 0.15's.
- **The Tool's admin read, under `deny`.** The tool reads `@/*/router` and
  runs S4, which reads clean. The caller, without the admin read, gets
  nothing.
- **security.md §3 step 3, as amended**, with `S` both ways:
  - `S-enrolled`, a principal holding only the open contract grants, is
    refused under both postures.
  - `S`, authenticated but no principal, is refused under `deny`. Under
    `allow`, its answer on R1's key arrives under its own replier id,
    unverified, and S4 is not clean.
- **A storage** must also end `…/storage_manager/storages/<name>`, beside
  the inclusion test (`live.check_s4`).

Implementing 0.15 raised no new question.

## At 0.16 (#609)

0.16 came from the reference's tools (`why`, `check conform`, `storage gen`,
`admin graph`), which zk2py never saw. This round reads its rules
cold. `just py-conformance` passes 519 of 519 (0.16 adds no fixture).
`just py-live` passes 232 of 232, with no known deviation.

**What zk2py built and showed:**
- **A tool's S1 check (§4.2)** is `live.s1_check`. It compares `meta.zid`
  with 0.13's outward set of verified routers, by value, then judges the
  stamp.
  - **An owner in client mode** under R1, which the tool verified, is
    clean.
  - **An owner whose own session is a router.** zk2py's owner opened in
    router mode and linked to R1, so R1's document lists it as a router.
    It is unobservable, neither clean nor a finding. The stamp alone would
    have read it as the owner's.
  - **Under `deny`**, the Tool, which holds the admin read, judges h1
    clean. The caller, without the admin read, verifies no router and
    reports unobservable (F-89).
- **O3 judged from outside (§5.1)** is `live.o3_verdict`, with the grants
  read from the deployment (`acl.may_call`) (F-90). Under `deny`:
  - the consumer, without the Call grant, gets silence, which is
    `unjudged`, not a finding;
  - the caller, with it, is answered: clean;
  - a granted call that `own-h2` holds past the caller's timeout is the
    finding, said to hold under grants that let the caller call.
  - **Building this exposed a bug in zk2py, not in the spec.**
    `CallResult.silent` counted zenoh's `Timeout` error reply as an answer.
    §5.1 says a call is silent with "no value and no envelope … the
    transport's own error reply included". Fixed.
- **§4.4 and U22.** zk2py's owner takes a tokenless set: `"token": false`
  in the descriptor, no interface token, no D code. An owner configured
  with `archive.v1` in it refuses to start and declares nothing. The test
  uses a stand-in contract with that interface id
  (`interop/stand-in/archive.v1.toml`), since the archive profile (#613) is
  not specified. Where to refuse is not in the text (F-91).
- **§2.6 retention.** zenoh-python routers carry no storage manager
  (`"plugins": null`), so whether the storage manager prunes by retention
  cannot be measured here. The text leaves the consumer's side to build,
  and state.md §8 says how: filter by the ULID's time. `live.replay_events`
  GETs with `_time=[now(-<retention>)..]` and filters by ULID. Against a
  stand-in storage that answers all 1,000 occurrences, ignoring `_time` as
  the memory backend does, it keeps exactly the 500 within the hour.
- **Appendix B.** zenoh-python 1.10.1 exposes `Reply.replier_id`. Its
  type stub marks it `@_unstable`, a marker only: the published wheel has
  it at run time, an `EntityGlobalId` whose `zid` is the replier's.
  - Without it, `live.replier_of` returns None, and every admin answer is
    unverified ("no replier id").
  - S4 is then unobservable, and so is S1 for an owner in client mode. The
    run simulates this (`live.READ_REPLIER = False`) and checks both.

### F-89 · ambiguity · §4.2 "A tool's S1 check" (0.16): a tool that verifies no router

**Status at 0.17: resolved by 0.17, partly against zk2py's guess.** §4.2 "A tool's S1 check": a foreign stamp is a finding whatever else the tool read. An owner's own stamp is clean only when the tool verified at least one router and `meta.zid` is none of the zids it knows to be routers. Otherwise S1 is unobservable. zk2py had held a foreign stamp unobservable when no router was verified, which 0.17 overturns: an owner that is its own router stamps with its own `meta.zid`. `live.s1_check` now judges the stamp first. The run shows a foreign stamp (an owner whose clock is another session's HLC) as a finding with routers verified, and again with none verified, by simulating a binding without the replier id.

> "A tool that reads the admin space (§11.1, the admin read) compares the
> owner's `meta.zid` with the verified routers' zids, by value. When they
> match, the owner is its own router, and the tool reports S1 unobservable
> for it, never clean."

The rule says what a tool does when it reads the admin space. It is
silent when the tool verifies no router: it lacks the admin read, the
admin space is off, or no replier id is readable. Two readings:
- **S1 is judged by `meta.zid` alone,** as before 0.16. An owner whose own
  session is a router the tool cannot see then reads clean, the outcome
  0.16 forbids.
- **S1 is unobservable.** Appendix B points this way for one cause:
  without the replier id, "the checks that read the admin space
  unobservable, never clean". Whether S1 counts among those checks for a
  tool that never read the admin space is not said.

**Resolved:** a guess. zk2py's `s1_check` reports S1 unobservable when no
admin answer is verified, whatever the cause ("the owner may be a router
this tool cannot see"). Under `deny`, the caller, without the admin read,
reports h1 unobservable, while the Tool reports it clean.

### F-90 · gap · §5.1 (0.16) and §11.3: a tool's own grants

**Status at 0.17: resolved by 0.17.** §5.1: a tool learns its grants from its operator or from the deployment's §11.1 input. A deployment without access control lets everyone call. Told so, a silence from an owner whose tokens it reads is the finding. Not told, it is unobservable, naming the premise it lacked. This is what zk2py had built, with one change: its verdict word for the untold case is now `unobservable`, where it was `unjudged`, and the finding needs the owner's tokens read (`live.o3_verdict(…, present)`).

> "A tool judging O3 from outside, as a conformance suite does, holds a
> silence as a finding only under grants that let it call: an
> access-control refusal is silent too (O5, §11.3). It says so beside the
> finding."

> §11.3: "the running configuration is not observable on the bus."

To hold a silence as a finding "only under grants that let it call", a
tool must know whether its grants let it call. It cannot observe them,
and the spec does not say where it learns them: from the deployment's
input, from the generated block, or from the operator. "It says so beside
the finding" also has a second reading: count every silence as a finding,
captioned with the condition, without knowing the grants. The two
readings differ for the case 0.16 is about, a caller without the grant.
**Resolved:** a guess. zk2py's `o3_verdict` takes `may_call` from the
deployment's generated grants (`acl.may_call`), and reports a silence as:
- a finding only when the grants let the caller call;
- `unjudged` when they do not, or when they are unknown.

### F-91 · gap · §4.4 (0.16): where an archive's tokenless configuration is refused

**Status at 0.17: resolved by 0.17.** §8.2 step 2 refuses an owner whose tokenless set names `archive.v1`, whether or not it implements it. A descriptor marking `archive.v1` `"token": false` is D011 (`descriptors/d011-tokenless-archive`, `ok-archive`). zk2py already refused at step 2, whatever the owner implements. Its checker now reports D011, and presence.md §2 step 6 runs through R1 against its control (F-92).

> "An archive MUST hold its `archive.v1` interface token: it is never in
> the tokenless set (§8.1)."

> §8.1: "A deployment MAY configure an owner with interfaces for which it
> holds no interface token."

The MUST binds the archive. Yet the tokenless set is the deployment's
configuration, and the core does not say what happens when that
configuration names `archive.v1`. Only the CHANGELOG says the reference
"refuses to start one". The core text leaves open:
- whether the owner refuses to start, or holds the token and ignores the
  configuration;
- where a refusal sits in §8.2's bring-up;
- whether a descriptor marking `archive.v1` `"token": false` is reported.
  No D code names it, so a checker reading that descriptor says nothing.

**Resolved:** zk2py's owner refuses to start, with step 2's other
refusals, before anything is declared. Its descriptor checker is
unchanged, because the D-code table has no code for this.

## At 0.17 (#609)

`just py-conformance` passes 521 of 521, with `descriptors/d011-tokenless-archive`
(D011) and `ok-archive`. `just py-live` passes 235 of 235, with no known
deviation.

What changed in zk2py:
- **A tool's S1 check** (`live.s1_check`) judges the stamp first. A foreign
  or missing stamp is a finding whatever else the tool read. An owner's own
  stamp is clean only with a verified router answer, and with `meta.zid`
  none of the zids known to be routers. Otherwise it is unobservable. The
  `0.16` run shows:
  - a client-mode owner, clean;
  - a router-mode owner, unobservable;
  - an owner's own stamp with no router verified (no replier id),
    unobservable;
  - a foreign stamp, a finding both with routers verified and with none.
  Under `deny`, the Tool with the admin read judges h1 clean, and the
  caller without it reports unobservable.
- **O3 from outside** (`live.o3_verdict`) uses 0.17's words. Told its
  grants let it call (by `acl.may_call`), a silence from a present owner
  is the finding. Not told, or told they do not, the silence is
  unobservable. Under `deny`:
  - the consumer's ungranted call is unobservable;
  - the caller's granted call is clean;
  - a granted call that `own-h2` leaves unanswered is the finding, and the
    same silence not told the grants is unobservable.
- **D011** is checked per entry, after cascades 2 and 3, and without the
  contract. presence.md §2 step 6 runs: an owner of `zk2py_needs.v1`, which
  does not implement `archive.v1`, with `archive.v1` in its tokenless set,
  refuses. The watcher through R1 sees no token, where the control (an
  empty set) showed its instance token.

### F-92 · contradiction · §3.3 D011 against "Cascades and scope" (0.17): `token` checked, and listed as unchecked

**Status at 0.18: resolved by 0.18, against zk2py's placement.** A new cascade 6: "D011 reads an entry's `iface` and `token`, and nothing else. It is reported for every entry whose `iface` is `archive.v1`, whether or not that contract is given, and whatever its `contract` says: cascades 3 and 5 do not suppress it." The old cascade 6, now 7, lists `token` as unchecked "except on `archive.v1`". zk2py had checked D011 after cascade 3, so the new fixture `descriptors/d011-bad-fingerprint` (`["D003", "D011"]`) failed with `["D003"]`. D011 now runs right after cascade 2, before the fingerprint test, and the fixture passes.

> D011: "an interface entry for `archive.v1` is marked `"token": false`: an
> archive is never tokenless (§4.4, 0.17) | error | per entry"

> Cascade 6, "Not checked, deliberately": "`minor`, an integer from 0 to
> 2^64−1 that no check reads …, and `token`"

D011 checks `token`, while cascade 6 still lists it as deliberately
unchecked. No cascade places D011 either:
- **Cascade 5** says an interface none of the given contracts declares "is
  checked for syntax only". Yet `d011-tokenless-archive` expects D011 with
  `archive.v1`'s contract not given, so D011 counts there as syntax,
  although it is a rule about one interface.
- **Cascade 3** says an entry whose `contract` is not a fingerprint "is
  checked no further". Whether D011 is still reported for it is not said,
  and no fixture decides.

**Resolved:** zk2py checks D011 for every entry whose `iface` is an
interface id and whose `contract` is a fingerprint, so after cascades 2
and 3, and before the contract lookup, so with or without the contract. It
reports D011 once per such entry, a repeated one included (cascade 4).

### F-93 · ambiguity · §4.2 (0.17) against (0.13): what "verified" means

**Status at 0.18: resolved by 0.18.** §4.2: an owner's stamp is clean only when the tool "counted at least one router's own answer", with "A counted answer, not a connection" stated. "Verified router" keeps 0.13's meaning. This is what zk2py built. `live.s1_check` now names its premise `counted` and quotes the new text.

> 0.13, "Verified routers, outward": "the routers the tool's session is
> connected to, and the session itself, are verified".

> 0.17, "A tool's S1 check": "An owner's stamp is clean only when the tool
> verified at least one router … and `meta.zid` is none of the zids it
> knows to be routers: the routers its session is connected to, the
> routers it verified, and every zid a verified router lists".

Read with 0.13's meaning, a client tool has always "verified at least one
router", its own, with no admin answer at all. With the admin space off,
it would then call an owner's own stamp clean, although the owner may be a
router it cannot see. 0.17's own list ("connected to … verified …
lists") treats connected and verified as two sets. Its "Otherwise"
names the cases where nothing is verified, and those include "the admin
space is off". So the intent is an admin answer verified, but the word
says the other.
**Resolved:** zk2py reads "verified at least one router" as at least one
router's admin answer verified ("Who answered"), and keeps 0.13's set
(connected, the session, and the routers listed outward) as the zids it
knows to be routers. The `0.16` run's simulated binding without the
replier id is the case that tells the two readings apart. It reports an
owner's own stamp unobservable there.

## At 0.18 (#609)

`just py-conformance` passes 522 of 522, with `descriptors/d011-bad-fingerprint`.
`just py-live` passes 235 of 235, with no known deviation: nothing moved.

What changed in zk2py:
- **D011 moved** before the fingerprint cascade (cascade 6, 0.18), which
  the new fixture required. The checker's docstring now lists the seven
  cascades.
- **`s1_check` quotes 0.18**, and names its premise "counted" answers
  rather than "verified" routers. Its logic is unchanged.

Implementing 0.18 raised no new question.

## At 0.19: hostid.v1 0.1 (#609)

Core 0.19 makes room for derivation-only profiles, and `hostid.v1` (text
0.1, draft) is the first profile. zk2py read both cold, with the Rust
runtime for hostid (chunk PB) being written at the same time. This round
built no Rust and ran no `just py-live`.

**`just py-conformance`: 565 of 565.**
- `descriptors/ok-derivation-profile` passes unchanged: zk2py's checker reads
  `profiles` for its form alone (D010).
- A new family, `hostid`, runs every file under
  `spec/profiles/hostid/conformance/`, as `profiles/README.md` asks of the
  second implementation. An unknown file there is a failure.
  - `vectors.json`: 28 cases, from §2.1 alone.
  - `shapes.json`: 14 cases, from §2.11 alone.

  All 42 passed the first time.

**The runtime half, offline** (`zk2py.hostid`). `Runtime(root)` is one
process's runtime over an injectable root:
- the input ladder of §2.4: absent, refused, unreadable (fails closed), and
  a non-regular file, read with a 4,096-byte bound;
- the shared file of §2.5:
  - a temporary file created exclusively, `fchmod` 0644, `fsync`;
  - then `link(2)`, a `fsync` of the directory, and the temporary file
    unlinked on every path;
  - `EEXIST` reads the winner's file as an input;
- `HostidError`, which names every path with its outcome and the OS error;
- the ephemeral opt-in, which replaces only "not created", and is logged;
- minting once per run, with the setting fixed by the first service that
  asks;
- §2.3's spelling, `address = "@hostid.v1/<service>"` and
  `hostid = { ephemeral = true }`, with its configuration errors.

zk2py's owner resolves `@hostid.v1` before its session opens. When the
system is minted, its descriptor lists `hostid.v1` and states `meta.host`
(§2.8, §2.13). For a tool, `hostid.minted_by_listing` answers §5's first
question, and `live.hostid_collision` §2.12's finding.

**The scenarios** (`python -m zk2py.hostid_scenarios`, `just py-hostid`):
29 of 29, in temporary roots. Where a section expects observations on the
bus, the bus is an in-process zenoh-python router R1 with zk2py's owners and
tool as its clients, not `py-live`'s Rust owner.
- **§1, the input order:** all five steps, and the privacy check over every
  sample, reply, token and descriptor the tool received.
- **§2, the racers,** as processes, which the text allows besides threads.
  - 16 spawned once, released by one barrier each round.
  - 100 rounds with no `var/lib/zk2`, and 100 with it present and empty.
  - Every round had one winner, one file of 32 hex digits and a newline, and
    no temporary file left. Every racer held the file's system.
  - 232 and 215 racers lost at `link(2)` with `EEXIST`, so the race path ran.
  - The 200 systems were distinct.
- **§3, fail closed:** all five steps, unprivileged. The runner is uid 1000:
  `chmod` makes the unwritable directory and the unreadable file, and
  `link(2)`'s `EPERM` is the runtime's seam. Each error names its paths with
  their outcomes, and through R1 no token or descriptor appears, where the
  control's does.
- **§4, ephemeral:** all five steps.
- **§5, once per run:** all five steps, with one caveat. zk2py's owner has
  no re-mint (core §8.1), so a new owner of the same address in the same
  process stands in for one. The new instance token appears before the old
  one goes, and the system is unchanged.
- **§6, a tool:** all four steps.
  - Two hosts from one image is the finding, its cause undecided.
  - The control is two systems, and no finding.
  - A missing `meta.zid` is unobservable.
  - A literal `h-` name is not minted.

**zk2py's live runner, against core 0.19.** `live_interop.py` asserted that
the Rust owner's `profiles` equals its contracts' `uses`. 0.19 makes that
true only for a literal system. The runner now expects the union, with
`hostid.v1` when the system it configured is `@hostid.v1`. Its runs
configure literal systems, so the expectation is unchanged for them.

**An implementation note, not a finding.** §2.4 makes a FIFO unreadable.
Opening one for reading waits for a writer, so a runtime that opens and
then checks hangs there. zk2py opens inputs with `O_NONBLOCK`. That is a
lesson for a guide (`profiles/README.md`: "Lessons go to guides, not
MUSTs").

### F-94 · gap · hostid.v1 0.1, scenarios.md "A root", and §2.4: a root as a seam, and symbolic links

> scenarios.md: "It runs each service against a root, a directory standing
> in for `/`, through a mount namespace, a chroot or a seam of the runtime
> under test."

> §2.4: "Symbolic links are followed: `/var/lib/dbus/machine-id` is often a
> link to `/etc/machine-id`."

A mount namespace or a chroot resolves an absolute link inside the root. A
seam does not, and the text offers seams to a runner. Under a seam, the
common link `/var/lib/dbus/machine-id -> /etc/machine-id` reads the real
host's machine id. A test that builds that link would then pass or fail on
the host it runs on. No step uses a link, so nothing shows it yet. The
text does not say whether a seam must resolve links inside the root, nor
that links are out of a seam's reach.
**Resolved:** zk2py's root is a seam that does not resolve links in it. Its
docstring says so, and a test that needs a link makes a relative one.

**Status at hostid 0.2: resolved by hostid 0.2.** §2.4: a runtime whose seam reads the inputs under another directory "MUST resolve paths there as a chroot would: an absolute link target starts at that directory, and `..` stops at it". zk2py's `Runtime.resolve` now walks each path component by component under the root, at most 40 links (`ELOOP`), and a non-directory component is `ENOTDIR`. Under `/`, the operating system resolves. The new steps §1.6 (an absolute link dangling in the root: absent, and M3's system whatever the host holds) and §1.7 (an absolute link reaching M2 in the root) pass.

### F-95 · ambiguity · hostid.v1 0.1 §2.3 and §2.7: when the first mint fails

> §2.7: "A process MUST mint its system at most once, when it starts the
> first service that asks for a minted system".

> §2.3: "One process, one setting. … It is fixed by the first service that
> asks for a minted system".

When the first service that asks fails closed (§2.6), nothing is minted.
Two questions follow, and the text answers neither:
- **The inputs.** May a later service of the same process read them again
  and mint, or is the process's one chance spent? Minting at the second
  service still mints "at most once", but not "when it starts the first
  service".
- **The setting.** Is it fixed by the first service, which never started, or
  by the first that minted?

No scenario runs a second service after a failure.
**Resolved:** a guess. In zk2py the first service that asks fixes the
setting, and a failure is not kept: a later service reads the inputs again.

**Status at hostid 0.2: resolved by hostid 0.2, as zk2py guessed.** §2.3 and §2.7: the first service that asks fixes the setting "whether or not it starts", and "a failure mints nothing, so a later service reads the inputs again, from the first". The new step §5.6 passes unchanged: `a` fails, the host is fixed, `b` starts with `h-bbd1aa1db10b`, and `c`, asking for the ephemeral rung, is a configuration error.

### F-96 · gap · hostid.v1 0.1 §2.5 step 4 and §2.6: the winner's file, gone

> §2.5: "on `EEXIST`, another racer won. The runtime reads the final file …
> and uses its content as an input (§2.4): an id, or, if it is now refused,
> absent or unreadable, a failure (§2.6)."

> §2.6: "Ephemeral … replaces exactly one refusal: every input gave no id,
> and the shared file was not created."

A refused or unreadable winner's file is what §2.6 calls a shared file
that holds no id, or an unreadable input. Both fail closed, ephemeral or
not. An absent one (removed between the winner's `link(2)` and this read)
is neither, and it is not plainly "not created" either: a file was created,
by another racer. So whether the ephemeral rung replaces it is not said.
**Resolved:** a guess. zk2py reports it as "not created" (`EEXIST, then
absent`), which the ephemeral rung replaces, since this runtime created no
file and none is there.

**Status at hostid 0.2: resolved by hostid 0.2, against zk2py's guess.** §2.5 step 4 and §2.6: a final file absent after `EEXIST` "was created by another racer and removed before it was read. It is not 'not created', so the ephemeral rung does not replace it". It is reported `absent`, one of §2.6's four outcomes, and "the step that found it says which absence it was". zk2py now returns the read's outcome, `absent` with the note "written by another racer", which fails closed. The new step §4.6, made through the `link` seam, passes.

### F-97 · ambiguity · hostid.v1 0.1 §2.12 against §5: who is counted

> §2.12: "Counting only instances whose descriptors list `hostid.v1`".

> §5, the first question: "Yes: Its descriptor lists `hostid.v1`, and no
> contract it implements lists it in `uses` … Unobservable: … a contract it
> implements lists `hostid.v1` in `uses`."

§2.12's count takes every instance that lists `hostid.v1`. §5 holds a
listing ambiguous when a contract the instance implements uses
`hostid.v1`, because core §3.3 then lists it for a literal system too.
§2.12 does not apply that caveat, so its finding can count an instance that
§5 cannot call minted. §2.8 makes such a contract a SHOULD NOT, so the case
is rare. But §2.12 and §5 answer the same question, and differently.
**Resolved:** a guess. zk2py's `hostid_collision` counts by the listing
alone, as §2.12 is written, and does not fetch the instances' contracts.

**Status at hostid 0.2: resolved by hostid 0.2, against zk2py's guess.** §2.12 now counts an instance "only when §5's first question answers yes for it". An instance that lists `hostid.v1` while that answer is unobservable makes the address unobservable, unless the counted instances establish the finding. `live.hostid_collision` now retrieves each instance's contracts by the fingerprints its descriptor names (§8.4), and answers §5's first question per instance. The new step §6.5 passes: both instances implement `zk2py_sysinfo_x.v1`, which lists `hostid.v1` in `uses`, so neither is counted and the address is unobservable. Live, the Rust owner example's instances are counted (first answer yes), through the same retrieval.

## At core 0.20 and hostid.v1 0.2 (#609)

Core 0.20 adds `self.system` providers (R1, R3), orders `profiles`, and
fixes a derived address before §8.2 step 1. `hostid.v1` 0.2 decides
F-94 to F-97. The Rust owner example now mints, with
`@hostid.v1/<service>`, `--hostid-root <dir>` and `--hostid-ephemeral`. Its
usage was read by running it. This round ran all three runners.

**`just py-conformance`: 565 of 565.** Neither text added a fixture.
`hostid` is 42 of 42, unchanged.

**`just py-hostid`: 34 of 34**, up from 29, with 0.2's new steps:
- §1.6 and §1.7, links under the root (F-94);
- §4.6, `EEXIST` and then the file absent (F-96);
- §5.6, a failure that fixes the setting (F-95);
- §6.5, a contract that uses `hostid.v1` (F-97);
- §5 step 2's `logger` binds `self.system/sysinfo`. Its descriptor lists
  `h-bbd1aa1db10b/sysinfo`.

**`just py-live`: 262 passed, 0 failed, 1 known deviation.** The 27 new
checks are the run `hostid`, against the owner example as a black box,
through a router R1 of the runner's:
- **The owner example minted over a root holding M1** is
  `h-bbd1aa1db10b/echo`. Its descriptor has no D code and lists `hostid.v1`,
  with `meta.host` and `meta.zid`. Its `profiles` are in §9.5's order:
  `a.v1, a.b.v1, hostid.v1, views.v2, views.v10`, from the new
  `interop/zk2py_order.v1.toml` (F-98). M1 is in it in no spelling.
  §2.12 is `no`: its contracts' `uses`, retrieved by fingerprint, do not
  list `hostid.v1`.
- **A zk2py consumer minted over the same root** has the same system,
  `h-bbd1aa1db10b`. It binds `self.system/echo` and `self.system/*`, and
  lists them resolved. Through `h-bbd1aa1db10b/*` its state GET answers the
  Rust owner's value, attributed by `meta.zid`. Through
  `h-bbd1aa1db10b/echo` the Rust owner's `@op/echo` answers. Both state
  the same `meta.host`.
- **The shared file, both ways.** On a root with no machine id, the
  first implementation creates `var/lib/zk2/hostid`: 32 lowercase hex
  digits and a newline, mode 0644, no temporary file left. The other
  reads it, and both hold the file's derivation. Rust created and zk2py
  read, then zk2py created and Rust read.
  - The two at one minted address are §2.12's finding, its cause
    undecided. Once one leaves, the answer is `no`.
- **Fail closed.** On a root whose `var/lib/zk2` is not writable, the owner
  example exits 1 with no token through R1, naming the three paths:
  `absent`, `absent`, `not created (Permission denied)`. With
  `--hostid-ephemeral` it starts on a system of the minted shape and writes
  nothing.
  - **The known deviation:** it says nothing about the ephemeral system on
    stdout or stderr, with or without `RUST_LOG` (F-99).
- **bindings.md §5,** with zk2py's detectors and trackers:
  - `vehicle-01/one` receives from `vehicle-01/det0` alone;
  - `vehicle-01/all` receives from both of `vehicle-01`'s detectors;
  - neither receives from `vehicle-02/det0`;
  - a put on a wildcard key reaches both trackers, and both discard it (R6);
  - the descriptors list `["vehicle-01/det0"]` and `["vehicle-01/*"]`;
  - a tool's `self.system/det0` is refused.

**What changed in zk2py.**
- **The hostid runtime** follows F-94 to F-97's resolutions (see their
  status lines). The ephemeral log is written at every start of a service
  whose system is ephemeral (§2.6, 0.2).
- **The owner** resolves `self.system` providers at start, after the
  address, from the same system (`owner.resolve_providers`), and lists them
  resolved (R3). A tool, with no system, gets a `ValueError`.
- `profiles` is sorted by `owner.profile_order`: by name as a string, then
  by major as a number (§3.3, 0.20).
- **The consumer's side:** `Owner.role_keys` and `Owner.subscribe_role` read
  through the resolved bindings. A sample on a wildcard key is discarded
  (R6). `Owner.publish` puts a stream sample.
- **Three interop contracts:**
  - `zk2py_tracker.v1`, a consumer of `zk2py_sysinfo.v1`;
  - `zk2py_sysinfo_x.v1`, which lists `hostid.v1` in `uses` on purpose
    (§6.5);
  - `zk2py_order.v1`, whose `uses` sort differently by whole string.

  Each loads with no finding, in zk2py and in the owner example.

### F-98 · gap · core 0.20 §3.3 and §9.5, with E020 (§9.2) and §10 point 2: two majors of one profile

> §3.3 (0.20): `profiles` "sorted as §9.5 sorts a contract's `uses`: by
> name as a string, then by major as a number".

> E020: "an annotation key not `<profile>.<key>` (split at the **last**
> dot; the profile is a `uses` name without its major …)".

0.20's order puts `views.v2` before `views.v10`, so it expects one name
with two majors. Across contracts that is plain: one contract uses
`views.v1` and another `views.v2`. Nothing says whether one contract's
`uses` may list two majors of one profile, though. E002 checks each
entry's form, and the canonical form removes exact repeats only. zk2py
and the owner example both load `zk2py_order.v1`, whose `uses` hold
`views.v2` and `views.v10`, with no finding.

Such a contract has an annotation key `views.<key>` with no major.
Appendix D's interim vocabularies are keyed by name too. Once a profile
publishes a vocabulary per wire major (§10 "Where profiles live": "one file
per wire major"), the text does not say which major's vocabulary such a key
belongs to, or which major W105 reads.
**Resolved:** a guess. zk2py refuses nothing, and checks an annotation
against the name alone, as E020 and Appendix D are written.
`zk2py_order.v1` carries no annotation, so the question does not arise in
its runs.

**Status at 0.22: resolved by 0.22, against zk2py's guess.** §10 point 2 and E002: "Two entries naming one profile at two majors are E002, reported once per profile". Keys stay major-free. zk2py had refused nothing. Its linter now reports E002 once per profile listed at more than one major, and the new fixture `contracts/e002-two-majors` passes. `zk2py_order.v1` tripped it, so its `uses` are split with a new `zk2py_order_b.v1`: `views.v10` and `a.b.v1` in one, `views.v2` and `a.v1` in the other. An instance implementing both lists the same union, which 0.22 allows (§3.3). The owner example and zk2py's consumer still list it in §9.5's order, `a.v1, a.b.v1, hostid.v1, views.v2, views.v10`.

### F-99 · gap · hostid.v1 0.2 §2.6 and scenarios.md §4 expected 1: where the ephemeral log goes

> §2.6: "It says so. At every start of a service whose system is ephemeral,
> the runtime logs that the system is ephemeral, with every path it tried
> and its outcome."

> scenarios.md §4, expected 1: "Each start logs that the system is
> ephemeral, and names the three paths with their outcomes."

The scenario expects a log that a runner can check. The text does not say
where it goes. A runner of another implementation's binary can read only
that binary's output, and the owner example writes nothing about it on
stdout or stderr, with or without `RUST_LOG`. Its fail-closed error is
on stderr, so its ephemeral log, if it is written, goes somewhere a black
box does not show. §2.6 is a MUST, and §4's step is a check that only an
in-process runner can make. The text does not say whether a runtime must
make the log observable to an operator by default, as the error is.
**Resolved:** a guess. zk2py's runtime keeps its log and hands it to a
`log` callable, and its own scenarios check that log. `py-live` reports
the owner example's silence as a known deviation, XFAIL, not as a
failure.

**Status at hostid 0.3: resolved by hostid 0.3.** §2.6: the ephemeral start goes "wherever the process's operational logs go, at its warning level or the equivalent", and a scenario reads it "through a log capture in process, or the process's own log output for a black-box runner". The owner example now writes its runtime's logs to stderr. py-live reads a `WARN` line there that names the three paths with their outcomes, and the check is a plain PASS, no longer XFAIL. The line carries terminal colour codes even on a pipe, which the runner strips. zk2py's runtime now logs through Python's `logging` by default (logger `zk2py.hostid`, at WARNING). Its scenarios' §4 step 1 reads the record there, through a handler.

## At 0.21–0.22: freshness.v1 0.1 (#609)

Core 0.21 publishes the first annotation vocabulary and makes a re-put a
mutation. Core 0.22 resolves F-98, and `hostid.v1` 0.3 resolves F-99.
`freshness.v1` (text 0.1, draft) is the second profile. zk2py read all four
cold, from `spec/` alone. The Rust examples were used as black boxes: their
usage was read by running them, and the owner example re-puts on its own
when its contract gives a `ttl_s`.

**`just py-conformance`: 653 of 653.**
- A new family, `freshness`, runs every file under
  `spec/profiles/freshness/conformance/`, from the text alone. An unknown
  file there fails.
  - `horizons.json`: 30 cases.
  - `judgements.json`: 57 cases, with every number of seconds read exactly
    to the nanosecond.

  All 87 passed the first time.
- `contracts/e002-two-majors` passes (0.22, F-98).
- W105 reads a profile's published table before its interim one (0.21),
  and says which it read. No code changes.

**`just py-freshness`** (`python -m zk2py.freshness_scenarios`, new):
21 of 21, on an in-process zenoh-python router R1 with timestamping on.
The owner, S and G are its clients, each with a session of its own.
- §1 to §5 are the runtime's, as the text says. §6, a tool's verdict per
  resource, runs zk2py's tool.
- The contract is `beacon.v1`, copied from the text into
  `interop/freshness/beacon.v1.toml`.
- G measures its clock from S's deliveries.
- §2 step 5's owner, 2 s ahead, watches a heartbeat that the runner puts
  through R1 without a stamp, so R1 stamps it. Its guard trips, a put is
  refused (`ClockAhead`), S receives nothing more and judges `status`
  stale, and its tokens stay present.

**`just py-hostid`: 34 of 34.** §4 step 1 now reads the ephemeral start's
WARNING through a logging handler (0.3).

**`just py-live`: 278 passed, 0 failed, 0 known deviations.**
- F-99's check passes, where it was an XFAIL.
- `zk2py_order.v1` is split, and §9.5's order still holds on both sides.
- The new run `freshness` adds 15 checks:
  - **The owner example serving `beacon.v1`** (`lab/beacon`) re-puts
    `status` (ttl 2) on its own. S, declared before it started, received 6
    deliveries in 5 s, at most 0.951 s apart. Each had the same payload and
    Encoding, was stamped by its `meta.zid`, and was stamped above the one
    before.
    - `intent` (ttl 0) and `note` (no horizon) are put once.
    - G's reply carries the latest re-put's stamp (core S2, 0.21).
    - S and G judge `status` fresh, G's clock measured from the owner's live
      puts.
    - Its bundle, retrieved by fingerprint, is zk2py's build. zk2py's tool
      reads it over 3 s: `status` fresh, `intent` fresh (`never_stale`),
      `note` not asked. `level` is unobservable: the example publishes no
      stream sample, so no member of it is known (§2.5).
  - **Its re-puts stopped.**
    - Stopped with SIGSTOP, it is fresh to S 1 s after its last delivery and
      stale 3 s after it. Its instance and interface tokens are still
      present, and both facts are reported (§2.11).
    - A GET of it is silent, so unobservable, and the member's combined
      verdict is stale (§2.7).
    - Continued with SIGCONT, it re-puts again, and S judges it fresh.
    - Closed, it re-puts nothing more and its tokens go, while S still
      measures the age: stale.
  - **zk2py's owner of `interop/freshness/zk2py_fresh.v1.toml`**
    (`lab/fresh`), read by the Rust `consume` example. Two reads 1.5 s apart
    show the same value under two stamps. Both are re-puts zk2py made, with
    zk2py's zid, the second later. Once zk2py closes the member's writer,
    two reads show one stamp.

**What changed in zk2py.**
- **`zk2py.freshness`**, new:
  - the session-free half: `horizon`, `judge_subscription`, `judge_get`,
    `judge_observation` (§2.7's order of checks), `combine`, `judge`, and
    `resource_verdict` (§5's second question);
  - the runtime half: `ClockTrust` (the deployment's word, or a
    measurement), `Subscriber` (a monotonic receive clock from its
    declaration, R6's discard), `get_reading` (S4), and `read_service`,
    the tool of scenarios §6.
- **The owner** re-puts every state member whose ttl is above 0, unchanged
  under a fresh minted stamp, once ttl/2 less a margin has passed since its
  last put (§2.4). Its GET answers then carry that stamp.
  - A deleted member, one whose writer closed (`close_writer`), and every
    member while the clock guard holds, are not re-put.
  - The guard is a router-stamped heartbeat subscription (core §4.3
    "Ahead", `heartbeat=`). While it holds, `set_state` raises
    `ClockAhead` (§2.10).
- `live.StateReply` carries the stamp's time in nanoseconds.
- **The linter:** E002 for two majors, and W105's published table.
- **hostid's log** goes through Python's logging.
- **New interop contracts:** `freshness/beacon.v1`, `freshness/zk2py_fresh.v1`
  and `zk2py_order_b.v1`.

**Implementation notes, not findings.**
- zenoh-python's own session replaces a stamp that is more than its HLC
  delta ahead of the session's clock, before R1 sees it. An offset
  simulated on top of the session's HLC, as §2 step 5 asks, therefore shows
  in the guard's comparisons, not in the stamps the owner's puts carry. On
  a real host the session's clock is the owner's, and the two cannot part.
- An owner re-putting exactly at ttl/2 shows gaps above ttl/2 at a
  subscriber, from scheduling and transit. zk2py re-puts a fifth of ttl/2
  early, at most 200 ms. The text allows it ("An owner MAY put more often"),
  and the reference's scenarios allow 200 ms of jitter.

### F-100 · contradiction · freshness.v1 0.1 §2.3 against §2.5 and `judgements.json`: a verdict with no value

> §2.3: "**No value, no verdict.** A reader that read no value of a member,
> a GET that drew no reply, has nothing to call fresh, whatever the
> horizon: unobservable (§2.6)."

> §2.5: "5. There was no delivery, and it has listened for longer than ttl:
> **stale**." and "At ttl 0, a member is fresh unless its last delivery is
> a delete (§2.3)."

> judgements.json: "ttl 0: never stale, no delivery needed (§2.3)", a
> subscription with no delivery, `fresh`, `never_stale`.

Read literally, §2.3 makes every reader that read no value unobservable,
"whatever the horizon". Yet §2.5 gives a subscriber that received nothing a
verdict both ways: stale after more than ttl of listening, and fresh at
ttl 0. The fixture cites §2.3 for the second. At ttl 0, a GET with no reply
is unobservable (`silent`, also in the fixture). So a subscriber that heard
nothing and a GET that drew nothing disagree, though neither read a value.
The reading that reconciles them makes "No value, no verdict" a GET
reader's rule: its example is a GET, and it cites §2.6. A subscriber judges
"a member it knows exists" (§2.5) instead of a value it read.
**Resolved:** the fixture decides. zk2py's subscriber judges a member it
heard nothing of as `stale` (`no_delivery`) past ttl and `fresh`
(`never_stale`) at ttl 0. Its GET reader gives `silent`, as
`judgements.json` does.

**Status at freshness 0.2: resolved by freshness 0.2, in wording, as zk2py read it.** §2.3: "A member that no observation showed a value for … has nothing to call fresh or stale, whatever the horizon: unobservable". §2.5's rules 5 and 6, and the ttl-0 line, "judge a member the reader knows": one that another observation, a GET's value, showed, whose silence since is the evidence. No expected value changed, and zk2py's judgements are unchanged. Its tool already judged only members a delivery or a GET made known (`read_service`).

### F-101 · ambiguity · freshness.v1 0.1 §2.7 and `judgements.json`'s description, against §2.3 and §5: nothing observed of a resource with no horizon

> judgements.json: "no observation at all is unobservable, no_observation."

> §2.7, the order of the checks: "For each observation, a reader asks in
> this order … 1. the resource declares no horizon: not asked (§2.3)".

> §5: "'Not asked' (nobody put the question) and 'unobservable' (it was
> put, and the observation could not be had) are different facts".

§2.7 asks its order per observation. With no observation, a resource's
absent horizon is never reached, and the description's rule answers
unobservable. §2.3 says that freshness is not asked of a resource without
a horizon, whatever was observed. §5's second question answers such a
resource "not asked" before it looks at any member. So a member of such a
resource that nobody observed is unobservable by the description and not
asked by §2.3, which §5 holds apart. No case pins it: every `no_horizon`
case has an observation.
**Resolved:** a guess. zk2py's member judgement follows the description:
no observation is `unobservable`, `no_observation`, whatever the horizon.
Its per-resource verdict asks the horizon first, so there a resource
without one is `not_asked`.

**Status at freshness 0.2: resolved by freshness 0.2, against zk2py's guess.** §2.7: "With no observation at all, steps 1, 2 and 4 still answer, since they need none: no horizon is not asked, an event or an operation is not this profile's, a value that is no horizon is unobservable." `judgements.json` gained four cases, three of which zk2py failed until `freshness.judge` asked the horizon's steps before combining. All 61 now pass.

### F-102 · gap · freshness.v1 0.1 §2.6, ground 2: how long a measurement holds

> §2.6: "2. **A measurement:** it received, live, a put stamped by that
> clock whose stamp was within the delta of its own clock at receipt."

One put within the delta grounds the trust. The text does not say for how
long, or what a later put by the same clock outside the delta does. It
could withdraw the trust, or do nothing, since one delivery can be late.
A reader that measured once at its start keeps trusting a clock that has
drifted since. §2.6's band then catches only a stamp ahead beyond δ
(`clock_disagrees`). A clock that fell behind makes its members read older
than they are, so stale. Scenarios §4 measures a clock that never agreed,
and §6 a window of 4 s, so neither shows it.
**Resolved:** a guess. zk2py's `ClockTrust` trusts a clock once any put by
it was within the delta, for the reader's life. It keeps the measurements
that fail, and does not act on them.

**Status at freshness 0.2: resolved by freshness 0.2, against zk2py's guess.** §2.6 ground 2: over one reading, "the clocks are trusted when the offset closest to zero is within the delta". "A stamp ahead of the reader's clock by more than the delta … withdraws the trust for the rest of the reading", and "a later offset above the delta does not withdraw it". "A reading is the span the reader judges in: a tool's window, or the lifetime of the subscription". zk2py's `ClockTrust.measured` now follows the rule, one object per reading, and the new fixture `clock-trust.json` (11 cases) passes. zk2py had trusted on any one put, and would have kept trusting through a stamp ahead beyond the delta.

## At 0.23: health.v1 0.1 (#609)

Core 0.23 says how a clock ahead is reported (a `faults` sample), that a
session with its HLC on stamps every put it is not given a stamp for,
where a standard contract lives, and that one revision may sit in two
history roots. `freshness.v1` 0.2 resolves F-100 to F-102. `health.v1`
(text 0.1, draft) is the third profile, and the first with a standard
contract. zk2py read all three cold, from `spec/` alone. This round ran no
`py-live` and built no Rust: the Rust health runtime is being written in
parallel.

**`just py-conformance`: 761 of 761.**
- A new family, `health`, runs every file under
  `spec/profiles/health/conformance/`. An unknown file there fails.
  - `judgements.json`: 61 cases.
  - `rollups.json`: 8 cases.
  - `codes.json`: 22 cases.

  All 91 passed the first time.
- `freshness` is 102. `clock-trust.json` (11 cases) passed once
  `ClockTrust` followed 0.2, and the 4 new judgement cases once `judge`
  did (F-101, F-102).
- The examples family walks the profiles' standard contracts too.
  `spec/profiles/health/health.v1.toml` loads with no finding. Its bundle
  is byte-identical to the one in `spec/profiles/.history`, and compatible
  with it. That root passes the §9.7 check. The revision held in both roots
  is one file, byte for byte (0.23).
  - `examples/zk2` has 4 checks fewer, since the contract moved: 93 there,
    6 for the profiles.

**`just py-health`** (`python -m zk2py.health_scenarios`, new): 28 of 28,
on in-process zenoh-python routers, in about 8 minutes.
- **§1, bring-up:** both of T's GETs, made the moment it saw the instance
  token and the `alive/health.v1` token, answer `UNSPECIFIED`,
  `"starting"`, stamped by the owner's session. `lab/quiet`, tokenless, has
  no `alive/health.v1` token, and its descriptor says `"token": false`. In
  35 s, one re-put arrived 29.8 s after the change. No delete, before or
  after the close.
- **§2, a stale status:** 65 s after the writer closed, S and G both judge
  `stale`, `beyond_horizon`, with the tokens present. G's reply is 65 s old
  against its measured clock.
- **§3, aggregation:** the status `FAILED` before `checks/disk` `FAILED`,
  and the check `OK` before the status `OK`. No delivery shows a status
  better than its check. A retired check is a `reply_del`, and no check is
  re-put. The owner refuses a status better than its worst current check.
- **§4, clock ahead:**
  - `clock_ahead` arrives within 1 s of the detection, and again 30.0 s
    later.
  - No status put is made from the detection on. At 65 s the status is
    `stale`, and the clock question answers yes.
  - Set right, the owner re-puts within 0.1 s of the release, and
    publishes no fault after. The verdict is `healthy`, and the clock
    question answers no.
  - The fault's stamp is the owner's session's, not R1's (F-103).
- **§5, a tokenless set of 100:** 200 tokens, none `alive/health.v1`,
  every descriptor `"token": false`, all 100 `healthy`. The tool's clock is
  measured from a subscription declared before the owners started (F-104).
- **§6, an absent owner:** `not_asked`, `absent`, and the stand-in
  archive's status shown as last-known, `DEGRADED`, with its stamp and
  `confirmed`.
- **§7:** `lab/liar` is `unhealthy`, `inconsistent`, at `FAILED` in both
  readings, a finding. `lab/frank` is `unhealthy`, `failed`, and agrees
  with its checks.
- **§8, a constrained face:** two routers side by side.
  - Run A: `nothing_crossed`, then `healthy` with no token crossing, then
    `stale`.
  - Run B: `face_closed` at each step, with nothing delivered.

**`just py-freshness`: 21 of 21.** **`just py-hostid`: 34 of 34.**

**What changed in zk2py.**
- **`zk2py.health`**, new:
  - the reader's half: levels with unknown kept apart, `judge` (§2.11),
    `rollup`, `code_class`, `status_agrees` and `clock_ahead` (§5), proto3
    codecs for the three messages, and `read_near` and `read_across` on a
    bus;
  - the owner's half, `HealthOwner`: the status before step 4, checks with
    §2.2's rule and order, faults of §2.10's form, and `clock_ahead` on
    the guard's first hold, every 30 s while it holds, and none after.
- **The owner:**
  - `initial=` values are put before the tokens;
  - a templated state member gets its publisher at its first put;
  - `on_guard` is told of each of the guard's transitions.
- **`zk2py.freshness`:** `ClockTrust.measured` follows 0.2, and `judge`
  asks the horizon's steps when there is no observation.
- **The examples family** walks `spec/profiles/**` against
  `spec/profiles/.history`, and checks the two roots' shared revisions.

### F-103 · gap (measured) · health.v1 0.1 scenarios.md §4 expected 2, with core 0.23 §4.1 and Appendix B: a clock ahead a runner cannot make

> scenarios.md §4: "an owner, `lab/ahead` … its clock 2 s ahead of R1 (the
> reference's `simulate_offset`)". Expected 2: "a `faults` sample with code
> `clock_ahead` and level `FAILED`, whose stamp is not the owner's (R1
> re-stamped a future-dated put, core §4.1)".

> v1.md §2.5: "A session with its HLC enabled stamps every put it is not
> given a stamp for … The fault therefore carries a stamp from the owner's
> clock, which is ahead."

R1 re-stamps the fault only when the owner's session HLC is itself ahead,
since the fault is published with no stamp of its own. A runner without
root cannot set the host's clock, and zenoh-python gives no way to move a
session's HLC. So zk2py, like any such runner, offsets the clock the owner
mints and guards with, over its session. Then:
- the fault, published without a stamp, carries the session HLC's own,
  which is not future-dated, and R1 keeps it with the owner's zid;
- given the offset stamp, the owner's own session re-stamps it before it
  leaves, with the owner's zid. This was measured with zenoh-python 1.10.1
  in the 0.21 round: an owner 2 s ahead put a state value, and it arrived
  with the owner's zid and an offset of about 0.

Either way R1 has nothing to re-stamp, and the expectation cannot be
met. The guard, the fault, its repetition and the stale status are all
observable. Only the stamp's origin depends on how the clock is made to
run ahead, and the text names `simulate_offset` without saying what it
moves. The text does not say whether a runner must put the owner's session
HLC itself ahead, or how.
**Resolved:** zk2py checks what holds however the offset is made: the
fault arrives with a stamp that is not future-dated. Its report names the
stamp's origin (the owner's session), and cites this entry.

**Status at health 0.2: resolved by health 0.2, with zk2py's measurement.** §2.5: "A simulated offset is not a drift": a session whose HLC runs re-stamps a put given a stamp ahead of it, "measured at 2 s ahead on zenoh 1.10.1, from Rust … and from Python". Nothing a reader concludes rests on the fault's stamp. scenarios §4 marks the R1 re-stamp **[moved clock]**, with a new step 4 under `drop_future_timestamp`. A runner meets that tier "with an owner that stamps from its offset clock on a session without an HLC". zk2py now runs both tiers:
- without root (`--only 4`), where it no longer checks the stamp;
- [moved clock] (`--only 4m`), with a client owner whose session has no HLC (`Owner(hlc=False)`), its faults stamped from its offset clock (§2.5's MAY, which zk2py now takes when the owner's clock is set apart from its session's). R1 re-stamps the fault with its own zid. Under `drop_future_timestamp`, S receives neither the fault nor the status, and G's reply holds the status under the owner's stamp.

py-live runs the same tier against the owner example under `--clock-offset-ms 2000`.

### F-104 · gap · health.v1 0.1 scenarios.md §5 to §7, with freshness.v1 0.2 §2.6: how the tool trusts its clock

> scenarios.md §5: "A tool lists `zk2/p5/*/@zk/**` … then GETs each
> instance's descriptor, and each listed service's `health.v1/state/**`,
> and judges." Expected: "judges each **healthy**, `ok`".

> freshness.v1 §2.6: "A GET reader MUST NOT age a reply unless it trusts
> that its clock and the clock that stamped the reply … agree within the
> delta", on "the deployment's word" or "a measurement" of live puts
> "during the reading it judges in".

The conventions give S and G a subscription, from whose deliveries G
measures its clock. §5's tool, and T in §6 and §7, read by GET alone. A
GET is no live put, so it measures nothing, and nothing says that the
deployment gives its word. Read as written, every status in §5 is
`clock_untrusted`, so unobservable, and §6's step 1 cannot be `unhealthy`.
A status re-put every 30 s would give a measurement only to a reader that
subscribes and waits that long.
**Resolved:** a guess. zk2py's tool subscribes to the statuses before the
owners start, as S does, and measures its clock from their first puts.
The deployment's word would also do on one host, and the text does not
say which of the two the scenarios mean.

**Status at health 0.2: resolved by health 0.2, against zk2py's guess.** The conventions and §5 to §7's setups: "T takes the deployment's word, `freshness.v1` §2.6's ground 1: every session of a scenario runs on one host … T measures nothing, and its verdicts rest on no subscription." Measuring was "the other way", not chosen because it ties those sections to a start order and a 30 s wait. zk2py's T now reads by presence and GET alone, with `ClockTrust(word=True)` (`Bus.t_read`), and §5 to §7 pass so.

## At health.v1 0.2: live against the Rust owner (#609)

`health.v1` 0.2 resolves F-103 and F-104, and the Rust owner example now
implements `health.v1`. zk2py drove it as a black box, by its flags and its
stdin commands, as `--help` and the coordinator's brief describe them. It
never read the example's source. The build used zk2py's own `target/`, with
line tables only, since the disk is tight.

**`just py-conformance`: 761 of 761**, unchanged: 0.2 changed no fixture.

**`just py-health`: 29 of 29**, following 0.2.
- §4 runs both tiers. Without root (`--only 4`) the stamp is not checked.
  [moved clock] (`--only 4m`) runs an HLC-less client owner, whose fault R1
  re-stamps with R1's zid, and step 4 under `drop_future_timestamp`.
- §5 to §7's T reads by presence and GET, on the deployment's word.

**`just py-live`: 303 passed, 0 failed, 0 known deviations.** The new run `health` adds 25 checks.
- **The owner example's health** (`--health`, `lab/svc`), read by zk2py:
  - a GET at its instance token answers its status, put before the
    tokens;
  - its descriptor lists `health.v1` in `interfaces`, and its `profiles` is
    `["freshness.v1"]`, without `health.v1`;
  - its bundle is revision 1.0, `sha256:e9dbcdcb…`, as zk2py builds it;
  - T, on the deployment's word, judges it `healthy`, `ok`, with
    `checks/disk` read.
- **The order of its puts** (§2.2), driven over its stdin, as zk2py's S saw
  it:
  - a check worse than the status: the status first, raised (`raised_by=`
    names the check), then the check;
  - a check better, or retired: the check first (or its delete), then the
    status;
  - at no delivery is the latest status better than a current check;
  - after each command, T's verdict follows: `unhealthy`, `failed` or
    `degraded`, then `healthy`, `ok`.
- **Faults:** an application's code is published, and `Bad-Code` is
  refused with §2.10 cited, nothing published. `status BOGUS` is refused.
- **Re-puts:** its status is re-put unchanged, 28.5 s after its last change.
  After `confirm off` it is stale at 65 s, never `FAILED`, with its tokens
  present.
- **Clock ahead** (`--clock-reference`, `--clock-offset-ms 2000`, the
  [moved clock] tier):
  - its first status reaches S re-stamped by R1;
  - on the heartbeat, `clock_ahead` at `FAILED` arrives, its detail naming
    2000 ms, under R1's stamp, and again 28.5 s later;
  - no status put is made while it holds, and at 65 s the status is
    `stale` and the clock question answers yes;
  - after `clock-offset 0`, the status is re-put within 0.8 s with no
    fault after: `healthy`, and the clock question answers no.
- **§4 step 4 under `drop_future_timestamp`:** S receives neither the fault
  nor any status put. G's reply holds the status under the owner's stamp.
- **The reverse.** The Rust `consume` example finds zk2py's health owner
  present, and reads its status: the protobuf bytes of `FAILED`
  "check disk: full", raised by the check, under zk2py's stamp. It then
  stops with `NoResource`, since `health.v1` has no operation for its call
  step, and it cannot name a member of `checks/{check}`.
  - Nothing on the Rust side judges health yet: `zenctl health` comes with
    chunk PF. zk2py could not check a Rust reader's verdicts on its owner.

**What changed in zk2py.**
- **`Owner(hlc=False)`** gives a client a session without an HLC, as a test
  rig, and `publish` takes a stamp. `HealthOwner` stamps faults from the
  owner's clock when it is set apart from its session's (§2.5's MAY).
- `_r1(drop_future=True)` sets `timestamping.drop_future_timestamp`.
- **The health scenarios** follow 0.2. `Bus.t_read` is T on the
  deployment's word, and `section4m` is the [moved clock] tier.
- **The live runner's Rust `Owner`** takes `flags`, and `command()` sends a
  stdin command and reads its one answering line.

**Observations, not findings.**
- **The two runtimes differ in API, not on the wire.** The owner example
  keeps a declared level and raises its status to the worst check by
  itself, lowering it when the check recovers. zk2py raises it, and leaves
  any lowering to the application. Both meet §2.2, whose "then the status,
  if it improves" allows either.
- **The `consume` example prints a protobuf state as its raw bytes,**
  lossily decoded as UTF-8. It is a test harness's printer, not a tool's
  rendering (core §7.2), so zk2py compares the bytes it can.

## At 0.24: the population budget (#609)

Core 0.24 adds §2.7, the population budget, with the fixture family
`budget/`. `health.v1` 0.3 gives §5's clock question a fixture,
`clock.json`. `hostid.v1` 0.4 fixes the wording of its scenarios §6. zk2py
read all three cold, from `spec/` and `examples/zk2/` alone, and drove the
Rust owner example as a black box.

**`just py-conformance`: 856 of 856.**
- A new family, `budget`, runs every file under `spec/conformance/budget/`.
  An unknown file there fails.
  - `bounds.json`: 15 cases.
  - `population.json`: 30 cases.
  - `rate.json`: 17 cases.

  All 62 passed the first time.
- `health` is 112, with `clock.json`'s 21 cases. All passed the first time,
  and the runner no longer reports `clock.json` as unknown.
- A new family, `layout`, checks that a family reads every entry of
  `spec/conformance/`. A directory added there, as `budget/` was, now fails
  the run until a family reads it.

**`just py-health`: 29 of 29.** Its §4 and the live run now answer the clock
question by `clock.json`'s rule, over what the subscriber heard in arrival
order (`health.clock_from`).

**`just py-hostid`: 34 of 34.** §6 already ran as a tool's scenario, all
five steps, with zk2py's tool (`live.hostid_collision`). Step 5's contract
is zk2py's own `zk2py_sysinfo_x.v1`, an id already spelled as 0.4 spells
it.

**`just py-freshness`: 21 of 21.**

**`just py-live`: 315 passed, 0 failed, 0 known deviations.** The new run `budget` adds 12 checks:
- **The owner example's `health.v1/state/checks/{check}`,** a templated
  state of cardinality 64. Its `--health-check` flags and `check` commands
  give it members, so it is the Rust owner's population zk2py judges.
  - Its descriptor states no `cardinality`, so the bound is 64.
  - A complete S4 GET answers 10 members, `within`, `complete`. Driven to 64,
    exactly the bound, it is still `within`.
  - It refuses a 65th check: "health.v1 holds at most 64 checks (§2.3)".
  - A retired check is a `reply_del`, not a member: 63.
  - Stopped with SIGSTOP, its GET ends at the timeout with an error reply,
    so it is incomplete: `unobservable`, `incomplete`.
- **zk2py's owners of `interop/zk2py_budget.v1.toml`:**
  - its state `items/{id}`: the descriptor lowers 4 to 3, the owner refuses a
    4th live member, and a complete GET is `within`. Once a member is
    deleted, a new one is accepted. A test owner that breaks the rule holds
    5, `exceeds`, `over_bound`;
  - its stream `samples/{id}`, cardinality 2: the conforming owner refuses
    a third member. Its window of seconds is `window_too_short`, since a
    complete one takes an hour. The test owner's three members are the
    finding in the same window;
  - its event `alarms/{id}`, cardinality 2, rate `low`, retention 60 s: the
    conforming owner refuses a second occurrence within a minute and a
    third member. A window of 61 s, with the owner present throughout, is
    complete: `within`, and the rate kept. The test owner's a twice and
    three members are both findings, in a window of a second.

**What changed in zk2py.**
- **`zk2py.budget`**, new:
  - `bound`, `population` and `rate`, as the fixtures read them, with
    integer nanoseconds;
  - on a bus, `get_population` (a complete S4 GET) and `Window` (a window's
    members on the receive clock, the owner's presence watched throughout,
    and a rate's spans on the stamps when they carry one clock id).
- **The owner keeps §2.7's rule:**
  - a new live member beyond the bound raises `BudgetExceeded`, for a
    templated state, a stream (live for an hour) and an event (live for its
    retention), as does an occurrence beyond its rate;
  - the descriptor states `cardinality=` lowerings, and `budget=False` is a
    test owner that breaks the rule;
  - `occur()` publishes an event occurrence on a fresh ULID.
- `health.clock_question` and `clock_from` implement clock.json's rule.

**Where zk2py's reading needed a guess.**
- **A lossless window** (F-105): zk2py reads "lost no delivery" as "knows of
  no lost delivery".
- **The owner present throughout:** zk2py declares a liveliness subscriber
  on the owner's instance token before it lists presence at the window's
  start. It requires the token at the start, no delete of it while
  listening, and the token at the end. The text says what to establish,
  not how; this follows from core §8.1, so it is no finding.

### F-105 · ambiguity · core 0.24 §2.7, "What a tool reads" and its table: a window that "lost no delivery"

> §2.7: "A complete reading of a stream or an event is a subscription
> window … It is complete when all three hold: … it lost no delivery, as a
> rate's clean pole requires (below)".

> The table, rate, clean: "a window at least one period long, no delivery
> lost". population.json: "`lossless`, false when it lost a delivery".

> freshness.v1 §2.5, rule 3: "It knows it lost deliveries (it fell behind
> and dropped some, for instance)".

A subscriber can know that it lost a delivery. It drops one itself, or
advanced publication's miss detection (core §2.5, `history`) reports a gap.
It cannot know that it lost none. A best-effort sample lost in transit,
or one a `drop` congestion policy discarded, leaves no trace at a plain
subscription. freshness.v1 states the condition as knowledge. §2.7 states
it as a fact. The two readings differ:
- **as a fact,** only a window with miss detection could ever be complete.
  A stream or event without `history` could never be clean, against 0.24's
  own reason for complete windows: "A rule that refused every window would
  leave a stream's population a question no reading could ever answer no
  to";
- **as knowledge,** a plain subscription that heard nothing amiss is
  lossless, even across a face that drops samples. A member whose samples
  were all dropped then reads as no member, so the window can read clean
  over a population above the bound.

**Resolved:** a guess. zk2py reads it as knowledge, as freshness.v1 §2.5
does. Its callback subscriptions drop nothing locally, so its windows are
lossless unless it knows of a loss. Its live event window ran on loopback,
with the event's reliable, `block` QoS.

**Status at 0.25: resolved by 0.25, against zk2py's guess.** §2.7: "**Lost no delivery is established, never assumed** … A window lost no delivery when both hold: 1. **Its reader lost none.** Its own queue neither overflowed nor fell behind, and no delivery was kept out of its count by a cap. 2. **The path could not drop one silently.** The resource declares `reliable` with congestion `block` … Otherwise no window of the resource is lossless, whatever it heard". "Miss detection is evidence of a loss, never of none." The fact reading is kept, and made reachable. A reliable, `block` path loses a delivery only by failing, and a failure withdraws the owner's token, which the presence premise reads until one lease after the window's end. zk2py's `budget.lossless` follows the rule, and the new fixture `lossless.json` (14 cases) passes. A best-effort window is now `lossy` at any length, where zk2py had called it lossless.

## At 0.25: "lost no delivery" established, and the owner keeping its budget (#609)

Core 0.25 resolves F-105, says how an owner keeps its bound and its rate,
and adds `scenarios/budget.md`. zk2py read it cold, and drove the Rust owner
example as a black box through its new knobs: the `--cardinality` flag, and
the `put`, `delete` and `fill` stdin commands, as its usage line and
refusals describe them.

**`just py-conformance`: 875 of 875.** `budget` is 81:
- the new `lossless.json`, 14 cases;
- `rate.json`, now 22, its readings carrying `lossless` and `present`;
- `bounds.json`, 15;
- `population.json`, 30.

All the new and changed cases passed the first time once `budget.lossless`
and `budget.rate(…, present)` followed 0.25.

**`python -m zk2py.budget_scenarios`** (new): 38 of 38. zk2py writes nothing
outside `impl/python/`, so this runner had no `just` recipe then (`py-budget` came later).
- **§1** used routers and an owner each in a process of its own
  (`python -m zk2py.proc`), and a consumer whose callback takes 2 ms per
  sample:
  - reliable, `block`: 1,000 of 1,000 arrived in order, with no withdrawal;
  - best effort, `drop`: 95 to 340 lost across runs, with nothing said;
  - reliable, `block`, the consumer stuck for 7 s on the 100th: 91 to 112
    lost, and the owner's token was withdrawn.

  The owner's puts did not visibly wait. The path's loopback buffers held
  the 16 MiB, so the 1,000 puts returned in about 0.05 s while the reader
  took 2 s. The text's waiting is the reference host's. Not reproduced, and
  no finding.
- **§2,** relays of this runner's, a 1 s lease on the routers and the
  consumer, the owner putting every 10 ms:
  - closed: the token withdrawn in 0.4 to 3.1 ms;
  - stalled: withdrawn after 962 to 1,010 ms, with 97 to 99 samples lost.

  This held between the routers and on the consumer's own link, and each
  link healed. The text measured 0.8–2.0 ms, and 986–1,037 ms with 88–93
  lost.
- **§3,** zenoh-python's `zenoh.ext` advanced publisher and subscriber.
  The first sample heard was the seventh, with no miss reported. After a
  cut, three puts and a heal, a miss of 15 or 16 was reported.
- **§4 to §7** were run on zk2py's owner and on the owner example, the same
  writes, 14 checks each. The same writes were accepted and refused.
  - Each refusal named the resource, the bound and its source (contract or
    descriptor), or the rate.
  - A subscriber saw nothing published for any refused write.

**`just py-health`: 29 of 29. `just py-freshness`: 21 of 21. `just py-hostid`:
34 of 34.**

**`just py-live`: 319 passed, 0 failed, 0 known deviations.** The `budget` run now has 16 checks,
reading windows by 0.25:
- zk2py's best-effort stream window is `lossy`, at any length;
- zk2py's event window of one retention, reliable and `block`, counted from
  its declaration plus the 1 s reply timeout, with the owner watched for 10 s
  and 1 s after its end, is `within`, `complete`, and its rate kept;
- the owner example's windows:
  - its event `alarms/{src}` (retention 2 s): `within`, `complete`. Its rate
    `burst(3/h)` is `window_too_short` in so short a window;
  - its reliable stream `load/{cpu}` is `window_too_short`, and its
    best-effort `samples/{id}` is `lossy`;
  - a second owner example whose process ended 2 s after its window's end,
    within the lease the tool watches: `owner_absent`.
- The owner example offers no lease setting, so the tool watches zenoh's
  default lease of 10 s.

**What changed in zk2py.**
- **`budget.lossless`** implements §2.7's rule and `lossless.json`'s order.
  `budget.rate` takes `present`.
- **`budget.Window`:**
  - it counts its span from its declaration plus the reply timeout;
  - it establishes losslessness from the resource's QoS;
  - it watches the owner's instance token from before its start until one
    lease and the reply timeout after its end, and it can be ended
    (`end()`) before that.
- **The owner's refusals** name the resource, the bound and its source, or
  the rate. A refused write publishes nothing, and its writer gets
  `BudgetExceeded`.
- **New:**
  - `zk2py.proc`, a router and a publisher in processes of their own;
  - `zk2py.budget_scenarios`;
  - `interop/budget/budget.v1.toml`, copied from the text with `minor` and
    `summary` added, so that it loads with no finding.

**Where zk2py's reading of 0.25 needed a guess.**
- **The window's start for presence.** zk2py requires the owner's token
  from before the window's declaration, which is stricter than from its
  counted start.
- **Members heard before the counted start**, between the declaration and
  the reply timeout, are counted. A member heard was live, and only the
  span is measured from the later instant.
- **The reader's own loss.** zenoh-python's callback handler has no queue,
  and zk2py's window keeps nothing out of its count, so its reader loses
  none.
- **The lease.** zk2py's in-process routers and owners, and the owner
  example, run zenoh's default 10 s. The tool watches 10 s after a window's
  end, as §2.7 says of the default.

## At 0.26: a late joiner's seed (#609)

Core 0.26 adds "A late joiner's seed" to §4.2: subscribe first, then S4's
GET, merged per key by the owner's stamp, and what a seed cannot know. Its
evidence is `scenarios/state.md` §10. `hostid.v1` 0.5 and `health.v1` 0.4
change citations only, in their Appendix B: nothing zk2py implements moves.
zk2py read them cold, and drove the Rust owner example as a black box (its
`put` and `delete` stdin commands). The example was built in the shared
target, and its copies were run.

**`just py-conformance`: 875 of 875**, unchanged. 0.26 adds no fixture: "No
pure function decides the merge apart from Zenoh's own timestamp order".

**`python -m zk2py.state_scenarios`** (new): 10 of 10, on an in-process
router. It needs a `py-state` recipe, which is outside zk2py's barrier.
- **§10, against zk2py's owner of `interop/zk2py_plans.v1.toml`.** The test
  holds the owner's answer to the GET (`Owner.state_hold`, `state_answer`,
  `answer_state`):
  - a is v2 with T4, and the reply's T1 was dropped by the merge;
  - c is v1 with T5, through the subscription;
  - b is a deletion with T2, the `reply_del`;
  - d is v1 with T3, once: the live put kept, and the equal-stamped reply
    the same mutation;
  - the seed is complete;
  - the control, GET first, misses c.
- **What a seed cannot know:**
  - a GET held past its timeout ends with an error reply, so the seed is
    possibly incomplete;
  - an owner absent at the GET seeds nothing, which is silence.

**`just py-budget`: 38 of 38. `just py-health`: 29 of 29. `just py-freshness`:
21 of 21. `just py-hostid`: 34 of 34.**

**The live runner, run directly: 326 passed, 0 failed, 0 known deviations.** The new run `seed`
adds 7 checks over the owner example's `budget.v1` items:
- a put made between the subscription and the GET is kept, once;
- a stale reply loses to a newer live put. The owner example answers a GET
  at once, so the runner holds the GET open with a queryable of its own on
  the same keys, which answers nothing and ends its part 1.5 s later.
  `Latest` then delivers the owner's reply at the GET's end, and the put
  made meanwhile arrives first;
- a delete reaches the merge as a deletion: the owner's `reply_del`;
- the seed is complete. With the owner stopped (SIGSTOP), the GET runs to
  its timeout, and the seed is possibly incomplete.

**What changed in zk2py.**
- **`zk2py.seed`**, new: `LateJoiner` (subscribe, then S4's GET, or the
  control's other order) and its merge. The merge goes per key by stamp,
  compared as `Latest` compares replies, and records each arrival as kept,
  the same mutation, or dropped. A consumer discards a key that is not
  concrete (R6); a tool keeps it.
- **The owner** lets a test hold a state GET (`state_hold`), make its
  answer, and send it later. Its ordinary answering path is unchanged.
- **New:** `zk2py.state_scenarios` and `interop/zk2py_plans.v1.toml`.

**Where zk2py's reading of 0.26 needed a guess.**
- **A sample with no stamp** (F-106).
- **Comparing two stamps of different ids:** zk2py orders by the NTP64
  time, then by the id read as an integer. The text says "as `Latest`
  compares replies"; the order of two ids at one instant is zenoh's, which
  zk2py does not read.
- **The Rust owner example cannot hold a GET's answer**, so the stale reply
  needed a queryable of the runner's own to keep the GET open. It answers
  nothing, so it holds no state on the owner's keys.

### F-106 · gap · core 0.26 §4.2 "The merge", with "A selector over several owners": a key with no owner, a sample with no stamp

> §4.2: "A late joiner MUST merge the GET's replies and the subscription's
> samples per key by the owner's stamp (S1, S2), whichever path brought
> each one, and keep the newer."

> "The keys it matches that no zk2 owner has, foreign keys (§1.7), are
> outside S4, and a storage may answer them."

> 0.26, "R6 and a tool's seed": "A tool's seeded view of the raw bus shows
> what arrived, as its live subscription does".

The merge orders by "the owner's stamp". A foreign key has no owner, and
its samples need carry no stamp. A session without an HLC puts unstamped,
and a router stamps such a put only where its timestamping is on (core
§4.1). A tool's seed over a selector that matches foreign keys, which 0.26
contemplates, can therefore hold samples the rule cannot order: an
unstamped live put against a storage's stamped reply, or two unstamped
puts. The text does not say which of these a seed does with such a
sample:
- keeps it as it arrived;
- orders it below every stamped sample;
- sets it apart.
**Resolved:** a guess. zk2py's `LateJoiner` orders an unstamped sample
below every stamped one. It replaces only another unstamped sample, the
later arrival winning, and never a stamped one.

**Status at 0.27: resolved by 0.27, as zk2py guessed, and its order of ids against zk2py's.** §4.2: "**An unstamped sample** (0.27) ranks below every stamped one, as `Latest` ranks an unstamped reply lowest (§4.1) … It is kept on a key that holds nothing, or only unstamped samples, and never over a stamped one. Two unstamped samples of a key have no order but their arrival, so the later replaces the earlier." That is zk2py's guess. "**R6 and a tool's seed** (0.27) … A tool's seed … keeps such a sample under the key it arrived on, merged with that key alone" is what zk2py did. The order was zk2py's other guess, which 0.27 overturns: "On equal times, by id: each id's 128-bit value written as 16 bytes little-endian, least significant byte first, and the two compared byte by byte from the first … at one time, id `100` (hex) is below id `2`." zk2py had compared ids as integers. `seed.Stamp.key` now compares the little-endian bytes. The new fixture `seed/merge.json` (7 orders, 13 cases) passes, and so do `state.md` §11's six steps.

## At 0.27: a reader that stalls, a tool as a role, the merge's open cases (#609)

Core 0.27 narrows §2.7's "its reader lost none" to a reader that did not
stall. It makes "Tool" a role whose grants compose with Own, adds a tool's
`inspects` and a provider's presence fan-in (§11.1, §11.2), and decides
the merge's open cases (§4.2), resolving F-106. zk2py read it cold, and ran
the Rust owner example as a black box (built in the shared target, copies
run).

**`just py-conformance`: 910 of 910.**
- `budget` is 95. `lossless.json` is reshaped: 21 cases with
  `reader_stall`, and 7 `stall_bounds`.
- The new family `seed` is 20: `merge.json`'s 7 orders and 13 cases.
- `layout` now knows `seed/`.

All the new and reshaped cases passed the first time once zk2py followed
the text.

**`just py-state`: 15 of 15.**
- §11 is new: W is a peer session without an HLC, the late joiner its
  client.
  - a unstamped, then replaced by the stamped reply;
  - c-1, c-2, then the reply's unstamped c-0, which arrived last;
  - b T2, a later unstamped delete not kept, and T3 kept;
  - at one time, id `2` above id `100` on e and f;
  - a tool's seed keeps `foreign/*` apart, and a consumer's discards it.
- The Rust owner example does not apply to §11: its keys are an owner's,
  always stamped by its session.

**`just py-budget`: 41 of 41.**
- §1 step 3: the consumer's callback stuck 7 s. 1 lost (the path's
  buffers held the rest), its own transport closed, the owner's token
  withdrawn, the ticker's gap 51 ms.
- §1 step 4, new: the consumer in a process of its own, which stopped
  itself (SIGSTOP) at the 100th for 7 s. 241 lost, the ticker's gap
  7,026 ms, its transport closed, the token withdrawn.
- "A tool's window over a stalled reader", new: an event of two sources,
  2 s retention, read over a 4 s window with a 1 s lease through a relay.
  - The relay closing the tool's connection mid-window: the window is not
    lossless (`reader_stalled`), and its population and rate are
    unobservable.
  - Left alone: within, complete.

**`just py-health`: 29 of 29. `just py-freshness`: 21 of 21. `just py-hostid`: 34 of 34.**

**The live runner: 329.** The `acl` run gains three checks:
- **inspection:** a tool inspecting `*/tc`, binding and calling nothing,
  reads both backends' tokens and descriptors on the doctor's wildcard
  selectors, complete. It reads none of their data, and no presence it
  does not inspect;
- **one principal, both roles:** `ops/both` puts under its own prefix and
  not on h1's key, and as a tool reads `*/tc`'s presence and the admin
  space;
- **§2's generator check:** without the presence selectors in the
  providers' egress, the inspector's wildcard descriptor read gets
  nothing, while its liveliness read still holds the tokens.

**What changed in zk2py.**
- **`budget.lossless`** reads the reader's stall: a transport of its
  session closed, or its ticker's gap beyond the bound, second in the
  order. `budget.stall_bound` derives the bound.
- **`budget.StallWatch`** watches zk2py's own receive path. It reads
  zenoh's transport events, which zenoh-python 1.10.1 exposes
  (`Session.info.declare_transport_events_listener`, unstable), and runs a
  ticker on a thread of its own. Every `Window` runs one, from before the
  window until the watch after it, so zk2py states no premise in place of
  transport events.
- **`seed.Stamp`** orders ids as zenoh does. `seed.compare` and `seed.kept`
  are the pure merge.
- **The ACL generator:**
  - a presence selector joins a provider's egress and no longer its
    ingress `reply`, which zk2py had granted;
  - a tool's grants compose with Own on one principal, as they already
    could;
  - `presence_egress` is §2's new generator check.
- **New:**
  - `proc.py consume`, a consumer that stops its own process;
  - `budget_scenarios`' section `1b`;
  - `state_scenarios`' §11;
  - `interop/budget/zk2py_stall.v1.toml`.

**Observations, not findings.**
- **zk2py's generator already put presence selectors on a provider's
  egress,** the fan-in 0.27 states in §11.1, "Inspection travels to the
  provider". It also put them on the provider's ingress `reply`, which
  0.27 rules out: "It does not join the provider's ingress `reply`: a
  descriptor is answered with a value, checked against its own key."
  zk2py no longer does.

**Where zk2py's reading of 0.27 needed a guess.**
- **The ticker's period:** zk2py ticks every 50 ms. The text bounds the
  gap, not the tick.
- **"No transport of its session closed"** covers every transport of the
  reader's session. A client has one, to its router.
- **The bound's inputs:** a `Window` takes the lease it is told (its
  session's), four keep-alives per lease (zenoh's default), and
  `wait_before_close` as 5 s unless told. The last is the router's, which
  the reader cannot read from its session.

## At config.v1 0.1: changing a running service's configuration (#609)

`config.v1` (text 0.1, draft) is the fourth profile: the configuration of a
running service, in groups classed hot, reach or contract, a change that
can cut the link applied only under a window and undone once unless
confirmed, persistence as a separate act, idempotency keys, redaction, the
startup allowlist, the change event and desired state. `freshness.v1` 0.3
corrected an appendix, and the profiles README's index lists `config.v1`.
zk2py read them cold, from `spec/` alone. Core is still 0.27. No Rust
owner of `config.v1` exists yet, so this round ran nothing across the two
implementations, and built no Rust.

**`just py-conformance`: 1209 of 1209.**
- A new family, `config`, runs every file under
  `spec/profiles/config/conformance/`. An unknown file there fails.
  - `validation.json`: 99 cases.
  - `window.json`: 18 cases.
  - `lifecycle.json`: 24 cases, each checked at its start and after each of
    its 113 steps: 137 checks.
  - `desired.json`: 28 cases.
  - `redaction.json`: 9 cases.

  All 291 passed the first time they ran. A mutation check confirmed the
  runner is not vacuous: an idempotency window of 299 s, a rate of 601, a
  deadline a millisecond late and a reversed order each fail cases.
- `examples` is 103: `config.v1.toml` loads with no finding, its bundle is
  byte-identical to the one in `spec/profiles/.history/config.v1/`, and it
  is compatible with it.
- `layout` is 17: it now fails a profile whose `conformance/` no family
  reads, so the next profile's fixtures cannot land unread.

**`python -m zk2py.config_scenarios`** (new; a `py-config` recipe is
requested): 51 of 51, against zk2py's owner of `config.v1`, through zk2py's
operation server and client, on in-process zenoh-python routers.
- **§1:** T's GETs at the instance token answer revision 0, nothing
  pending, the floor 10, every group's `writable`, `pin` without a value.
  The hot set's reply is the read-back at revision 1, with `startup` 10000.
  One `applied` event, actor `ops`; V and St re-put within 30 s, unchanged,
  and never deleted.
- **§2:** the pending reply before the apply (the device's apply 1.000 s
  after the reply's arrival); a confirm before `apply_at` is `busy`; the
  confirm after it is kept, its event attributed to step 1's
  `request_id`; a second confirm is `not_found`.
- **§3, §4:** the floor raises a 5 s window to 10 s; the undo at the
  deadline, once; a refused reach apply on `lab/rf1` ends `rolled-back`
  with no undo and revision 0; a failed undo ends `partial`, the value
  unknown, never retried.
- **§5 to §7:** `busy` naming the token, joins in order with no event, one
  `confirmed` with three groups' edits; `extend` from now, the undo 80 s
  after the set within 0.5 s; a restart is a new instance at revision 0
  from the startup file, and the late confirm `not_found`.
- **§8:** under zk2py's generated `deny` block, `persist` granted to
  `admin` reads `overlay` and survives the restart; withheld from `ops`,
  it draws nothing, never `not_found`; an owner without a persisted layer
  answers `unavailable`, cause `config`, and lists it so.
- **§9 to §12:** the allowlist before the values; a dropped reply's retry
  gets the first answer and no second apply, another request under the key
  is `invalid_request`, and at 300 s the key is new; a sensitive value is
  never carried, its device words left out, its persist `forbidden`, its
  log line `"<redacted>"`; desired documents converge, or are refused
  whole as `reach` or `sensitive`.
- **§13:** across `constrained.md` §1 step 2's attachment, built as
  `health.v1` §8's (C a client of the owner's router, which denies `@zk`
  and the state subscription to C's principal, C's batch at 1 KB), the
  pending reply is 118 bytes, the confirm's status 265 (the view would be
  1139), and the status's `instance` shows the restart. No `@zk` delivery
  crossed. The link is not shaped to a serial line.
- **§14:** zk2py's §5 reader answers no, not asked, yes, not yet, no
  (`rolled-back`), clean, "will not survive" with `startup` 10000, not asked
  with the owner absent, and unobservable past the horizon without
  presence.

**The other runners.**
- `py-budget`: 27 of 27, §4 to §7 on zk2py's owner alone. The owner
  example's copy went with round 27's `target/` and was not rebuilt, so its
  14 checks did not run.
- `py-health` 29 of 29, `py-state` 15 of 15, `py-freshness` 21 of 21,
  `py-hostid` 34 of 34.
- The live runner's zk2py-only runs pass, 99 of 99: refusal, s1, bringup,
  presence-refused, o1, fanout, tool-rules, 0.16 and acl. Its runs over the
  Rust examples were not run this round (no Rust build).

**What changed in zk2py.**
- **`zk2py.config`**, new: the session-free half (time, values, the orders
  of refusals, desired state, redaction, `Core`), `ConfigOwner`,
  `SimDevice`, and the reader of §5.
- **`zk2py.config_scenarios`**, new, and the author contract
  `interop/config/zk2py_author.v1.toml`, which copies `DesiredConfig`.
- **The owner:** `occur` published a JSON Schema event with the encoding
  `application/protobuf`, a zk2py bug no earlier event had met: it now
  writes the type's encoding (§7.2). `OpCall.metadata()` reads core O7's
  call metadata, and `live.call` sends it as the attachment.
- **The ACL generator** granted an event's template key, not its
  occurrences (`…/events/change/<ulid>`, core §2.6): a zk2py bug, fixed;
  no earlier deployment used an event.
- **The runner's `layout`** checks the profiles' directories.

### F-107 · ambiguity · config.v1 0.1 §2.5, with core §5.1 "The request" and `SetRequest`: where a value outside `ParamValue` is refused

> §2.5: "**A request outside its type** (a `confirm_s` of 0, an empty
> `token`, a `values` that is not an object) is refused `invalid_request`
> before row 1: by the core's decode, or by the owner's own check of the
> schema's bounds where its decode does not hold them (core §5.1, "The
> request")."

> §2.5: "**A value of the wrong kind** is any JSON value outside the
> parameter's kind: `1` for a `bool`, `"1"` for an `integer`, `true` for a
> `text`, `null` for any. … a value beyond 64 bits is out of every
> parameter's bounds."

> §2.5: "row 11 takes them in the bytewise order of their names (UTF-8),
> and the first problem is the same everywhere."

> core §5.1: "An owner MAY decode into a type generated from the schema, or
> checked against it, and need not evaluate the JSON Schema itself; the
> reference does not."

`SetRequest.values` is `additionalProperties: ParamValue`, and `ParamValue`
is `anyOf` boolean, integer, string. So `null`, `1.5` and an integer beyond
64 bits are outside the request's type. A decode into a type generated
from the schema (an untagged boolean, `i64` or string) refuses them before
row 1. An owner that checks the request's structure and leaves the values
to row 11 refuses them there, as the text lists them. The text knows the
difference for `2.0`, which JSON Schema accepts ("That is stricter than
JSON Schema 2020-12 … it is the owner's further check"), but not for the
values JSON Schema itself refuses.

The two readings answer differently whenever an earlier row applies:
- `set/power` (no such group) with `{"values": {"x": null}}` is
  `invalid_request /values/x` by the decode, `not_found group` by row 3;
- the same with an act in flight is `invalid_request` against `busy`;
- with a change pending, `invalid_request` against `busy` again.

No fixture pins it: `validation.json`'s `null`, `1.5` and 2^63 cases have
no earlier row applying.
**Resolved:** a guess. zk2py's decode checks the request's members and
their types, `values` an object, and leaves every value to row 11, as
§2.5 lists them.

**Status at config.v1 0.2: resolved by 0.2, against zk2py's guess for the values, and its order of members as zk2py guessed.** §2.5: "Outside its type are: … a value in `values` that is not a `ParamValue` (§3): `null`, a number with a fractional part, an array, an object. A decode that holds the schema refuses these, so every owner refuses them here, before any row, whatever its decode;" and "several at once: the first in the bytewise order of the member names, `values` last, and in `values` the first in the bytewise order of the parameter names." And "`2.0` and `2e0` are not integers of the kind, though JSON Schema 2020-12 reads them as integers, and so of the request's type", with "a value beyond 64 bits, an integer of the request's type", stay at row 11. zk2py's decode now refuses a non-`ParamValue` value (`decode_set`, `is_param_value`), `values` last. The 8 new `validation.json` cases pass; round 28's code failed the 4 that place a value before rows 1, 3, 4 and 5. Live, the owner example answers the same 37 probes zk2py predicts, these included.

### F-108 · ambiguity · config.v1 0.1 §2.5 "An empty values", against §2.3 and §2.11: a change of nothing

> §2.5: "**An empty `values`** is a change of nothing. It is accepted, and
> applies nothing to the device; its change, event and revision are as any
> other's."

> §2.3: "**The revision** counts the acts that moved the running values in
> this instance: an owner MUST add one at each apply and each undo (§2.9,
> §2.11), and at nothing else."

> §2.11: "A change none of whose groups is applied yet … has nothing to
> restore: no act is made, the revision does not move, and the change ends
> `rolled-back`."

A change of nothing moves no running value, so §2.3 counts no act, while
§2.5 makes its revision "as any other's", one more. "Applies nothing to
the device" says no device act is made, or a device act with nothing in
it. Its undo, under a window, has nothing to restore: an act or not? No
lifecycle case has an empty `values`.
**Resolved:** a guess. zk2py makes the apply as an act with no values (the
device is called), moves the revision by one, mints a token and publishes
the event with no edits. Under a window its undo has no applied parameter,
so no act is made and the revision does not move.

**Status at config.v1 0.2: resolved by 0.2, as zk2py guessed.** §2.5: "An empty `values` is a change of nothing. It is accepted, and its apply is an act that sets no value: the owner makes it, and the device has nothing to change. Its change, its event, with no edits, and its revision are as any other's (§2.3). Its undo, under a window, has nothing to restore, and is no act (§2.11)." §2.3: "an owner MUST add one at each apply, an empty one included (§2.5), and each undo". `lifecycle.json`'s new case 24 passes; round 28's code passed it too.

### F-109 · gap · config.v1 0.1 §2.3 and §2.11, lifecycle.json case 4: a value not read back, and its source

> §2.3: "`value`: the running value. It is absent … for one whose value the
> owner cannot read back: absence is unknown, never a guessed value;" and
> "`source`: where the running value came from".

> §2.11: "the owner shows each parameter the change set as it reads it back
> from the device, and with no value, unknown, where it cannot." … "A
> parameter read back after a failed undo shows the value read: with the
> source it had before the change when that is the value it had before,
> and `runtime` otherwise."

The text gives a source to a value read back, and says nothing of the
source of one that is not. `lifecycle.json`'s case 4 ("an undo that fails
ends partial; a value not read back is unknown") shows that parameter with
no `source` at all (its `params` entry is `{}`).
**Resolved:** by the fixture. zk2py shows an unknown value with no source.

**Status at config.v1 0.2: resolved by 0.2, as the fixture said.** §2.3: "It is absent with the value when the owner cannot read the value back (§2.11)"; §2.11: "such a parameter has no `source` either (§2.3)". Live, the owner example's view after a failed undo, the device reporting nothing, shows the parameter with neither (§4).

### F-110 · gap · config.v1 0.1 §2.17, lifecycle.json case 19: the edge of the rate's hour

> §2.17: "It keeps it by refusing `busy` a `set` that would start a change
> while 600 change events were published within the last hour (§2.5, row
> 12)".

`lifecycle.json`'s case 19 holds 600 events at t = -3000 s: a `set` at
t = 599 is `busy`, and one at t = 600 is accepted. So an event exactly an
hour old no longer counts. The text says so of the idempotency key ("At
300 s and after, the key is forgotten"), not of the rate.
**Resolved:** by the fixture. zk2py counts the events in (now - 1 h, now].

**Status at config.v1 0.2: resolved by 0.2, as the fixture said.** §2.17: "The hour is core §2.7's span: an event published exactly an hour before no longer counts."

### F-111 · gap · config.v1 0.1 §2.13 "After it", with validation.json: persist under another pending change

> §2.13: "**After it.** Each parameter the change set reads `source:
> overlay`, and `startup` leaves it (its startup value is now its running
> value)."

> §2.11: "The restored parameters read their values and sources from
> before the change."

`validation.json` accepts "persist the last change while another is
pending". When that pending change set a parameter the last change set
too, the parameter runs the pending change's value. Its startup value is
then not its running value, and `source: overlay` would claim a runtime
value comes from the persisted layer. If the pending change is then
undone, §2.11 restores the value "from before the change", with the source
`runtime`, though that value is now the persisted one, and `startup`
would vanish while the source says `runtime`.
**Resolved:** a guess. zk2py writes the last change's values to the
persisted layer. A parameter the pending change holds keeps `runtime` and
shows the persisted value as `startup`. Its value before the pending
change, when equal to the persisted value, restores with `source:
overlay`.

**Status at config.v1 0.2: resolved by 0.2, zk2py's guess kept for an applied parameter, and a branch it had not covered decided.** §2.13: "What it writes. The values the last change set, as it set them: never a value a pending change runs since, which a restart must roll back (§2.12)." After it: "a parameter an applied group of the pending change set keeps its running value and its source, and shows the value written as its `startup`. If that change is undone, the parameter returns to its value before the change, at `source: overlay` when that is the value written (§2.11);" and "a parameter only a reach group not yet applied set still runs the value written, and reads `source: overlay` at once." zk2py had left that last parameter at `runtime` until the pending change ended: round 28's code failed steps 9 and 10 of the new `lifecycle.json` case 25. `Core.persist` now marks it `overlay` at once.

### F-112 · gap · config.v1 0.1 §2.3 "Two keys, one revision": a pair the revision cannot name

> §2.3: "**Two keys, one revision.** The view and the status are two keys,
> and no two keys change at once (core §2.1). A reader that reads both
> pairs them by `instance` and `revision`."

> §2.3: "A confirm, an extend, a persist, a join that has not yet applied,
> a dry run and a refusal leave it." "It MUST put both on every change of
> either: a value, a source, the floor, the pending change (armed, applied,
> joined, extended, ended), the last change, the desired report."

The revision counts acts, not versions of the documents, so one
(`instance`, `revision`) names several documents in turn. Arming a reach
change, a reach join, an extend, a confirm, a cancel before any apply, a
persist and a desired report each re-put both at the same revision
(`lifecycle.json` case 2: revision 0 without and then with `pending`). A
reader that pairs a view without `pending` with a status at the same
revision showing one, or a view at `source: runtime` with a status after
the persist, pairs two moments as one. The text gives no member that
tells two documents of one revision apart, and no rule for which pair a
reader may take.
**Resolved:** a guess. zk2py's reader takes each document by itself, judged
by its own freshness, and pairs nothing. It puts the view before the
status on every change.

**Status at config.v1 0.2: resolved by 0.2, as zk2py's reader does.** §2.3: "An owner SHOULD put the view first, then the status. … So `instance` and `revision` name the running values, never a pair: a reader that reads both pairs them only when the status is the view's projection (§2.19), and otherwise takes each by itself, judged by its own freshness."

### F-113 · gap · config.v1 0.1 §2.9, §2.5 row 13: a starting reach change the device cannot apply in time

> §2.9: "`apply_at` is the device's to state. The owner MUST state it, and
> MUST state one before the deadline … When its device cannot apply it
> before the deadline, the `set` is refused `busy` (§2.5, row 13), and the
> caller extends the change first."

> §2.5, row 13: "Not a dry run, the group is `reach`, the request joins,
> and the group could not apply before the change's deadline (§2.9)."

Row 13 covers a join. A reach `set` that starts a change has a deadline at
least the floor away, and nothing requires the floor to exceed the time
the device needs before it can apply (the floor "is an instance fact, at
least 1 s"). A device whose `apply_at` would fall at or after the deadline
leaves the owner two MUSTs it cannot both keep, the device's `apply_at`
and one before the deadline, and no row says what it answers.
**Resolved:** a guess. zk2py refuses to construct an owner whose apply
delay is not shorter than its floor, so every window has room for its
`apply_at`.

**Status at config.v1 0.2: resolved by 0.2, zk2py's refusal at construction now the rule.** §2.8: "An owner MUST keep its floor above the longest delay its device states for a reach `apply_at`, from the `set`'s receipt (§2.9), so every change it starts can apply before its deadline. … Row 13 of §2.5 is then a join's alone".

**Where zk2py's reading of `config.v1` 0.1 needed a guess, beyond the
findings.**
- **The decode's own order.** When several members of a request are
  outside their types, zk2py names the first in the bytewise order of the
  member names, `values` required. A request that is not an object is
  refused with the pointer `""`. The core leaves the decode to the server.
- **"An equal request"** (§2.14: "every member of `SetRequest` equal"):
  a member absent and a member at its default (`dry_run: false`) are not
  equal, and values compare with their kinds (`1` is not `true`, `2` is
  not `2.0`).
- **Time written to the millisecond** (§2.8): an instant finer than a
  millisecond is truncated, not rounded.
- **The binding of a desired document** (§2.18 defers it: "`desired.v1`,
  a later profile, does"): zk2py's owner reads one key its configuration
  names, by subscription and by a GET at start, acts on a document only
  when its revision changes, and converges again in a new instance.
- **"Did my change land?"** (§5): a change answered by an instance that is
  gone counts as rolled back only when the tool has no event of it; zk2py
  does not tell a windowless change from a pending one there.
- **The log** (§2.15, scenarios §11 step 4): zk2py's owner logs each
  decoded `set` as one JSON line, its sensitive values `"<redacted>"`.
- **§10's dropped reply** is dropped at the owner, after it is kept as the
  first answer. **§13's face** is the attachment of `constrained.md` §1
  step 2 without a shaped link, as `health.v1` §8's was.

**Observations, not findings.**
- **The fixtures' `field`** ("group", a JSON pointer, or null) is what the
  envelope's `message` names, and core §5.2 has no member for it ("The
  `message` is for a person, never parsed"). It is checkable inside an
  implementation, as zk2py's `Refusal.field`, and never on the wire.
- **§2.5's integer rule needs a parser that keeps `2` and `2.0` apart.**
  Python's does; a reader that decodes every number as a double cannot
  apply it.
- **scenarios §3 step 2** reads "`St`'s `frequency_hz` is 868100000 again,
  at `source: file`": the status carries no source, so zk2py reads the
  source from V.

## At config.v1 0.2 and core 0.28: the answers, and `config.v1` live (#609)

Core 0.28 defines every error code (`forbidden` among them, §5.2), lets a
handler refuse one member of an exposed operation with `unavailable`
(§5.1), and holds an event's rate across a restart through a record the
owner keeps (§2.7). `config.v1` 0.2 answers F-107 to F-113, and states what
the reference's runtime settled: an unwritable persisted layer refused
before the request's type, the rate across a restart, and convergence
waiting for an act in flight. zk2py read both cold, from `spec/` alone, and
ran `config.v1` against the Rust owner example's `--config`, built in this
round's own target (`target-py`), as a black box.

**`just py-conformance`: 1240 of 1240.**
- `errors` is 45: the three new cases (`forbidden` decodes, a cause on it
  and a detail on `busy` are refused) passed as written.
- `config` is 319: `validation.json` 107 (8 new), `lifecycle.json` 26 cases
  and 131 steps (157 checks, 2 cases new), the rest unchanged. Every case
  passed once zk2py followed 0.2 (F-107, F-111).

**`just py-config`** (the recipe now exists): 51 of 51, against zk2py's
owner.

**Live: zk2py the caller, the tool and the subscriber, the owner example
the owner** (the live runner's new `--only config` run, 94 of 94):
- **its bring-up:** the view and the status, GET at its instance token,
  the status the view's projection; its descriptor lists `config.v1` with
  the standard contract's fingerprint and `freshness.v1` alone, with no D
  code; its bundle is revision 1.0 as zk2py builds it; its schema is
  `lifecycle.json`'s `rf` and a `modulation` contract group;
- **the order of refusals across the two:** 37 requests to an idle owner,
  decode faults and rows 3 to 11 and the control operations' and
  `persist`'s, each predicted by zk2py from the view the owner serves
  (§2.5 "A caller that checks first") and answered by the owner example.
  Every answer matched, code, cause and `ConfigError` code, and nothing
  changed;
- **`scenarios.md` §1 to §14, all of them run, 51 of 51**, in
  `config_scenarios`' owner-example mode (`--owner <exe>`), driven by its
  flags and its stdin:
  - §3, §4, §11 through `config fail-apply` and `config fail-undo
    unknown`; the device's acts read as counts from `config device`;
  - §7, §8 step 4 and §13 step 4 through `config restart`, which re-mints
    the instance in its process and keeps the persisted layer and the
    event record;
  - §10's 300 s through `config offset 300000`, the owner's clock moved, as
    the reference's test does; its lost reply is set aside unread by C,
    since the owner example cannot drop one;
  - §12 through `config desired <json>`, the owner example's binding;
  - §8 and §13: the owner example takes no credentials, so it sits on an
    open router linked to C's router. §8's router runs under `allow` with
    `ops`'s `persist` denied rather than zk2py's generated `deny` block;
    what C reads is the same.
- **No section was not runnable.**

**The live runner over the Rust examples** (round 28 had skipped them):
423 passed, 0 failed, 0 known deviations of the Rust owner example, the config run included.

**The other runners.** `py-budget` 41 of 41 (§4 to §7 on zk2py's owner and the owner example), `py-health` 29 of 29,
`py-state` 15 of 15, `py-freshness` 21 of 21, `py-hostid` 34 of 34.

**What changed in zk2py.**
- **`zk2py.config`:** `decode_set` refuses a value outside `ParamValue`
  before any row, `values` last (F-107); `Core.persist` follows §2.13's
  "After it", and refuses an unwritable layer before the request's type
  (`layer_writable`); resolved findings are cited by section.
- **`zk2py.config_scenarios`:** each section drives its owner through an
  adapter, zk2py's `ConfigOwner` or the owner example (`--owner`).
- **The live runner:** `--only config`, `run_python_config`.

### F-114 · gap · config.v1 0.2 §2.18 with §2.12: a bound document after a restart

> §2.18: "It tries a failed document again when its revision changes."

> §2.18: "The view and the status carry `desired`: the `observed_revision`
> of the last document the owner acted on".

> §2.12: "Runtime changes are not persisted (§2.13), so a restart during a
> pending change rolls it back by construction." … "**A restart is a new
> instance** (core §1.5): a new instance id, the revision from 0, nothing
> pending, no last change, and no idempotency key held (§2.14)."

The values a document converged are changes without a window, runtime
values, so a restart rolls them back to the startup values. The
deployment still binds the same document, at the same revision. The text
does not say whether the new instance converges that revision again, its
values now differing from it, or whether a revision acted on once stays
acted on across the restart ("each revision once", as 0.2's changelog
says of the reference and zk2py within an instance). Nor does it say what
the new instance's `desired` reads before it acts: absent, as if it
converged nothing, or the old instance's report.
- The owner example, whose binding is its stdin, holds no document after
  `config restart`: its status has no `desired`, and the parameter runs its
  persisted value, not the document's.
- zk2py's owner reads its bound key again at start, and converges.

0.2's changelog names the neighbour case, "A converged document after a
later `set`", as deliberately not stated; a restart is not a later `set`.
**Resolved:** a guess. zk2py's new instance acts on the bound document as
on a first one: it converges it, and reports no `desired` until it has.

**Status at config.v1 0.3: resolved by 0.3, as zk2py guessed.** §2.18: "**After a restart** (text 0.3, F-114). The binding is the deployment's, and survives a restart; the values a document converged were runtime changes, which the restart rolled back (§2.12). So the new instance acts on the document bound to it as the first instance did, converging it again, and its view and its status carry no `desired` until it has acted: a new instance knows nothing of the old one's report." And "**Each revision once, in an instance** (text 0.3)", which zk2py already did within an instance.

### F-115 · ambiguity · config.v1 0.2 §2.5 "A request outside its type", with `SetRequest` and `ControlRequest`: an integer member written `30.0`, or beyond 64 bits

> §2.5 (0.2): "`2.0` and `2e0` are not integers of the kind, though JSON
> Schema 2020-12 reads them as integers, and so of the request's type." …
> "a value beyond 64 bits, an integer of the request's type, is out of every
> parameter's bounds."

> §2.5: "Outside its type are:" … "a `confirm_s` of 0, a `token` or an
> `idempotency_key` empty or longer than its bound (§2.6, §2.14), a
> `values` that is not an object;"

> §2.8: "`confirm_s` is an integer number of seconds, at least 1."

> core §5.1: "An owner MAY decode into a type generated from the schema, or
> checked against it, and need not evaluate the JSON Schema itself".

`SetRequest` declares `confirm_s` and `expected_revision` `{"type":
"integer", "minimum": …}`, and `ControlRequest` its `confirm_s`. By 0.2's
reading of `values`, `"confirm_s": 30.0` and `"confirm_s": 2^64` are of the
request's type. No row reads them (row 11 reads `values`), so an owner
whose decode checks the request against the schema accepts a 30 s window,
or one of 2^64 s, and checks rows 1 to 14 after it.
- **Measured:** the owner example refuses `confirm_s` and
  `expected_revision` written `30.0`, `0.0` or 2^64, and `extend`'s
  `confirm_s` `30.0`, at its decode ("invalid type: floating point
  `30.0`, expected u64"), before row 1: `set/power` with `confirm_s: 30.0`
  is `invalid_request`, not row 3's `not_found`. zk2py refuses them at its
  decode too.

The two implementations agree. The text names neither the integer rule
for these members nor their 64-bit range, and an owner that reads them
as JSON Schema does would answer otherwise.
**Resolved:** a guess. zk2py refuses them at the decode, with the member's
pointer, as the owner example does.

**Status at config.v1 0.3: resolved by 0.3, as zk2py and the owner example both did.** §2.5: "a member of the request other than `values` that JSON Schema reads as an integer and core §3.3 does not: `confirm_s` or `expected_revision` written with a fraction or an exponent (`30.0`, `3e1`), or beyond the unsigned 64-bit range. Unlike a value in `values`, whose kind only row 11 knows, each of these members has one type, the request's, so it is read as core §3.3 reads an integer, and the decode refuses it (text 0.3, F-115)". zk2py's bound was the signed range; it now reads the unsigned one (`U64_MAX`). The 7 new `validation.json` cases pass, and live, the owner example answers `confirm_s: 30.0`, `expected_revision: 2^64` and `extend`'s `confirm_s: 60.0` as zk2py predicts.

**Where zk2py's reading of 0.2 and of the owner example needed a guess.**
- **F-114 and F-115** above.
- **§8 and §13 against the owner example** run on an open router linked
  to C's, since it takes no credentials; §8 under `allow` with one deny,
  not the generated `deny` block.
- **§10 against the owner example**: the lost reply is set aside unread
  by C, and the 300 s are the owner's clock moved.
- **The device's acts** are compared as counts, which both owners report,
  so §2's "the device's record of the apply is later than the reply's
  send" is read as no apply at the reply's arrival and one after.

**Observations, not findings.**
- **The owner example's descriptor** gives its `unavailable` entry a
  `reason`, core §3.3's optional member; zk2py's §8 check first required
  an exact entry, and now reads `resource` and `cause`.
- **The owner example repeats a plain parameter's value** in a refusal's
  message ("got 1e+300"), which no rule forbids, and never a sensitive
  one: `pin` given `2.0`, `1e300`, 2^96 or `4321` is refused naming `pin`
  alone.

## At link.v1 0.1, config.v1 0.3 and core 0.29 (#609)

`link.v1` (text 0.1, draft) is the fifth profile: what crosses a
constrained face. Its contract word is `link.exposure`; its deployment word
is a face policy, one TOML file per near router, from which a generator
derives a plan per face (named denies, one downsampling rule per concrete
key, the attachment, the window floors, the liveness evidence). Core 0.29
publishes its vocabulary (`exposure` alone; `link.downsample_ms` draws
W105), makes a crossing counter's epoch a SHOULD, and makes §8.5's batch
example exact. `config.v1` 0.3 answers F-114 and F-115, and settles a hot
group left part applied, a re-mint while a configuration runs, a converged
revision after a later `set`, and "an equal request". zk2py read all three
cold, from `spec/` alone.

**`just py-conformance`: 1373 of 1373.**
- `link`, a new family, is 119: `exposure.json` 31, `plan.json` 11 (two of
  them zenoh-modem's faces from its own contracts) and `lints.json` 77.
  Every case passed the first time it ran. A mutation check (no overlap,
  no templated-stream rule, `lora` a known protocol, the default window
  at 3600 s) fails cases; the carve-out's `**` written once before `/**`
  is the one rule no case pins (below).
- `contracts` is 99: `w105-link-downsample` passed once zk2py's
  vocabulary table followed Appendix D (0.29).
- `config` is 330: the 7 new `validation.json` cases passed as zk2py
  already read them (zk2py's F-115 bound moved from the signed to the
  unsigned 64-bit range, which no case tells apart), and lifecycle case
  26, "equal as decoded", passed once zk2py read `"dry_run": false` as
  absent: 0.3 overturned zk2py's reading there, which 0.2's changelog had
  named open.
- `layout` is 18: it failed on `spec/profiles/link/conformance` until the
  `link` family read it, as round 28 made it do.

**`python -m zk2py.link_scenarios`** (new; a `py-link` recipe is
requested): 22 of 22, every section run, on zenoh-python routers. N
listens on TCP (the near side) and on a `unixsock-stream` path (the face),
with usrpwd and the block zk2py's generator compiles under `deny` from the
enrollment (`zk2py.acl`), the plan's denies in the far principal's policy
(§2.4), the plan's `downsampling` block beside it, and N's `gateway` for a
south region.
- **§1:** the block holds the plan (the denies as `plan.json`'s third
  case derives them, two rules at 1 Hz). Over the face G receives
  `air/dropped_total` and `frames` 5 times each in 5 s, nothing of
  `air/frames_total` or `raw`, all 10 `oper` puts, all 5 `fault`
  occurrences and no `change`; L receives 100 of each stream. G's GET and
  call are answered by grant. L sees the tokens, G none and no descriptor.
  The block checks clean.
- **§2:** F, a far router in a south region `ground`, attaches over the
  face; G is its client and receives what §1's G does. With the far
  principal granted `declare_queryable` egress on the owners' queryables
  its GET and call are answered; without the grant neither is; granted on
  its bindings' selectors alone, the call is answered and the GET is not
  (F-117). `attach = "router"` is refused.
- **§3:** one rule per key, 5 samples from each owner; one wildcard rule, 5
  in all (3 + 2), and the block check reports `wildcard_downsampling`; a
  service not listed crosses at its owner's rate, 100 samples.
- **§4:** 5 `fault` occurrences cross, none of `change`; with the sketch's
  `…/stream/change/**` deny, all 5 of `change` cross; with
  `expose = "none"`, none.
- **§5:** a 6 s window across a 4 s outage answers a `reply_del` for both
  deleted members, and the far merge removes them; at 2 s neither is
  answered, both stand, and the linter reports `window_under_outage`.
- **§6:** live at 5 s over `radio.v1/state/oper` and
  `radio.v1/stream/air/dropped_total`; unobservable 6 s after the owner
  closes; unobservable from the start with `expose = "none"`.

**config.v1 0.3.** `just py-config`: 51 of 51. Live, against the Rust owner
example's `--config`, rebuilt this round: the live runner's `--only config`
run, 97 of 97, the order-of-refusals probes now 40 with three of 0.3's.

**The other runners.** `py-state` 15 of 15 and `py-health` 29 of 29 (the
owner's tombstone window became configurable). The rest were not rerun:
nothing they read changed.

**What changed in zk2py.**
- **`zk2py.link`**, new: the face-policy parser and linter, exposure, the
  plan with §2.7's carve-out, the windows' and the block's checks, a
  generator for zenoh-python routers (`compile_face`, `router_extras`), and
  §5's liveness reader.
- **`zk2py.link_scenarios`**, new, with `interop/link/radio.v1.toml`, the
  scenarios' contract.
- **`zk2py.config`:** "equal as decoded", the unsigned 64-bit range of the
  request's integer members, and a hot group left part applied restored
  once (a device result `{"refused": words, "partial": {…}}`, zk2py's).
- **The contract loader:** `link` moved to the published vocabularies,
  `exposure` alone.
- **The owner:** `tombstone_s`, the window core S3 lets a deployment set.
- **The live runner's config probes:** three of 0.3's.

### F-116 · gap · link.v1 0.1 scenarios.md "Conventions" and §5, with core §11.1 and §11.2: subscriptions wider than any grant

> scenarios.md, Conventions: "**The generator** compiles N's
> `access_control` block, under `deny`, from the enrollment below" … "a
> principal `ground` that binds every stream, state and event of
> `radio.v1` from both" … "Each subscribes to `zk2/lab/*/**`,
> `zk2/lab/*/radio.v1/@stream/**` and `zk2/lab/*/radio.v1/events/**`, and
> holds a liveliness subscriber on `zk2/lab/*/@zk/**`, before the owner
> starts."

> scenarios.md §5: "G seeds: it subscribes to
> `zk2/lab/radio-a/radio.v1/state/**`, then GETs it as core S4 says."

> core §11.2: "**Egress is checked by inclusion** against the query's or
> subscription's own key expression."

A binding's grants name its resources, so G's and L's grants are each
resource's key expression at each provider. Measured on a zenoh-python
1.10.1 router under `deny`, with allows on `a/b` and `a/c`: a subscriber on
`a/**` receives nothing, a GET of `a/**` or `a/*` is answered by nothing,
and a GET of `a/b` is answered. A declaration or a query that no grant
includes is refused, ingress as egress. So G and L, as the conventions
write them, receive nothing under the block they say the generator
compiles, and §5's GET of `state/**` is answered by nothing. No grant a
binding compiles to includes `zk2/lab/*/**`, and the providers' egress
grants carry the consumers' selectors, not these (core §11.2). The text
does not say how the reference's G subscribes, or whether its enrollment
grants something wider.
**Resolved:** a guess. zk2py's G and L subscribe to exactly the selectors
the enrollment grants them (each binding's, at each provider), hold a
liveliness subscriber on each provider's `@zk/**`, and §5's G seeds by a
GET of each granted state selector.

**Status at link.v1 0.2: resolved by 0.2, as zk2py measured and did.** scenarios.md, Conventions: "Each subscribes to the selectors its grants name, one per stream, state and event resource of `radio.v1`, `zk2/lab/*/radio.v1/<token>/<pattern>` … Under `deny`, zenoh admits a subscription or a GET only when a grant includes its key expression (core §11.3): a reader subscribed wider than its grants, to `zk2/lab/*/**`, would receive nothing at all, so the readers subscribe exactly to what they are granted (0.2, F-116)." §5: "G seeds: it subscribes to its granted selectors over `lab/radio-a`'s state, `…/radio.v1/state/oper` and `…/radio.v1/state/peers/*`, then GETs each as core S4 says (0.2: never `state/**`, which no grant includes, F-116)." zk2py's readers now take their selectors from the example's enrollment, `lab/*` where zk2py had named each radio.

### F-117 · gap · link.v1 0.1 §2.5, restating core §8.5, with core S2: the far principal's queryable grant

> link.v1 §2.5: "**A south region needs declarations sent toward it.** The
> far router learns of a queryable only from a declaration, so core §8.5
> requires the far principal to be granted `declare_queryable` on egress
> for what its side may query."

> core §8.5: "the far side's principal MUST be granted `declare_queryable`
> on egress for what its side may query. Without that grant, its queries
> are never routed."

> core S2: "The owner's state queryables cover its interfaces' `state/**`
> and `@state/**`."

The declaration N sends toward the far router is the owner's queryable's,
and the access control checks it by inclusion (core §11.2). An operation's
queryable is its own key, which "what its side may query" includes. A
state's is `…/<iface>/state/**`, which the far side's bindings
(`…/state/oper`, `…/state/peers/*`) do not. Measured in scenarios.md §2:
with the far principal granted `declare_queryable` egress on its bindings'
selectors, G's call is answered and its GET of `peers/x` is not; granted
on the owners' `state/**` and `@state/**` as well, both are; with no grant,
neither. "What its side may query" names the queries, while the grant must
include the owner's declarations, which are wider.
**Resolved:** a guess. zk2py's §2 grants the far principal
`declare_queryable` egress on its bindings' selectors and on each owner's
`state/**` and `@state/**`, and records the narrower grant's result as a
measurement.

**Status at link.v1 0.2 and core 0.30: resolved by both, as zk2py measured and guessed.** §2.5: "core §8.5 requires the far principal to be granted `declare_queryable` on egress over every queryable its side may query, as the owner declares it (core 0.30)", covering "the owner's state queryables over its interface's `state/**` and `@state/**` (core S2), with the providers as bound" and "the operation's queryable over its template's pattern (core §5), `@op/set/*` for a call of `set/duty`". Core §8.5: "MUST be granted `declare_queryable` on egress over every queryable its side may query, as the owner declares it (0.30, measured)". scenarios.md §2 adds zk2py's control, step 3: "G's call is answered and its GET is not: a declaration is checked by inclusion against the queryable's own key expression, which the GET's selector does not include (core §11.3)." zk2py's grant is now `link.declaration_grant`, and §2 runs the three grants.

**Where zk2py's reading of link.v1 0.1 needed a guess, beyond the
findings.**
- **A face whose name is not a plain chunk:** its keys are still checked
  (no case has one with a bad key).
- **`shared_protocol`** is read over the faces whose `link_protocols` is
  well formed and names known protocols.
- **`batch_over_lease`** is computed whenever the batch, the rate and the
  lease are well formed, whatever else the policy holds.
- **The windows' check** runs only when the plan is derived, and the plan's
  `service` is the service's address, as the fixtures and `lints.json`'s
  description write it (the text's table says "service").
- **The generator's rule ids** are `face-<face>-<deny id>`, added to the
  far principal's policy beside zk2py's generated grants.
- **§1's traffic:** the streams for 5 s and `oper` over 9 s, one put a
  second, concurrently.

**Observations, not findings.**
- **The carve-out's `**` written once** ("`R/x/*/**`, never
  `R/x/*/**/**`") is the one rule of §2.7 no fixture pins: a deny of a
  subtree under a rest parameter never arises in `plan.json`.
- **zenoh-python 1.10.1 carries every piece the text needs:** a
  `unixsock-stream` face with usrpwd, downsampling selected by
  `link_protocols` (2 puts in 2 s at 1 Hz, measured), and `gateway` and
  `region_name` for a south region. A router with usrpwd credentials for
  its own outgoing link authenticates its incoming clients too, so §2's G
  is a user of F's.

## At link.v1 0.2 and core 0.30 (#609)

`link.v1` 0.2 answers F-116 (the scenarios' readers subscribe to their
grants) and F-117 (the declaration grant covers the owner's queryables as
it declares them), fits the owner's traffic in its 5 s, and points at the
scenarios' radio, now an example (`examples/zk2/link/`: the contract, its
enrollment and the lab router's face policy). Core 0.30 spells §8.5's
declaration grant out the same way. No fixture changed. zk2py read both
cold, and four questions the reference had to answer by reading.

**`just py-conformance`: 1377 of 1377.** `examples` is 107:
`examples/zk2/link/radio.v1.toml` loads with no finding, its bundle
byte-identical to `examples/zk2/.history`'s, and compatible with it. zk2py's
linter reads `radio.face.toml` and `walkthrough.face.toml` with no finding.

**The example's files against zk2py's copies.**
- **`radio.v1.toml`:** the body is the one zk2py had copied from
  scenarios.md, byte for byte; only the header comment differs. zk2py's
  copy under `impl/python/interop/link/` is removed, and the scenarios read
  the example.
- **`radio.face.toml`:** the policy zk2py had built from scenarios.md, the
  same keys and values. The scenarios now read it, each section's changes
  applied to it.
- **`radio.enrollment.toml`:** zk2py had built its principals in code,
  binding each radio by name (`lab/radio-a`, `lab/radio-b`). The example
  binds one tool on `lab/*`, run by `local`, `ground` and
  `ground-router`. zk2py reads it into its principals (a small reader of
  the reference's format, which core §11.1 leaves to the generator), so
  the readers' selectors are now `zk2/lab/*/radio.v1/<token>/<pattern>`,
  as 0.2 writes them.

**`just py-link`** (the recipe now exists): 23 of 23, all six sections as
0.2 writes them, and one measurement for F-118.
- **§2's three grants**, the far principal's `declare_queryable` egress
  over:
  - **none** (step 2): neither G's GET nor its call is answered;
  - **G's own selectors**, `zk2/lab/*/radio.v1/state/peers/*` and
    `zk2/lab/*/radio.v1/@op/reset` (step 3): the call is answered, the GET
    is not;
  - **0.2's grant**, `zk2/lab/*/radio.v1/state/**`, `…/@state/**` and
    `…/@op/reset` (step 1): both are answered.
- **§1, §3 to §6** as in round 30, with `oper` every half second: G
  receives 5 samples of each crossing stream, all 10 `oper` puts, 5 `fault`
  and no `change`; one rule per key; the sketch's event deny leaks; a 6 s
  window answers the deletes across a 4 s outage and a 2 s one loses them;
  liveness live, then unobservable, never down.

**What changed in zk2py.**
- **`zk2py.link.declaration_grant`**, §2.5's grant (0.2).
- **`zk2py.link_scenarios`** reads `examples/zk2/link/`, runs §2's three
  grants as 0.2 numbers them, and measures F-118.

**The four questions, read from the text alone.**
1. **§2.4, which subjects "can match a session on the face".** By zenoh's
   rule as §2.4 quotes it ("a session matches every subject whose properties
   all match it"), every subject none of whose properties excludes the
   face's transport: the far principal's, and any other principal's
   selected by its user name or CN alone, since zenoh ties no credential to
   a link. §2.4 then sets the near principals aside by a fact of the
   deployment ("connect elsewhere"), which nothing in the plan or the
   router enforces. The text decides the far principal's, and leaves a near
   principal on the face undecided: F-118.
2. **§2.15, whether rules in two separate `downsampling` items can
   "intersect".** No. §2.8 makes a face's downsampling one block, one item
   (`{id: <face>, link_protocols: …, rules}`), and §2.15 checks "a router's
   block for one face", its rules "in the block's order". Two items are two
   faces' blocks, and §2.14 keeps their link protocols disjoint, so no link
   meets both. zk2py's checker takes one face's block. The text says
   nothing of a hand-written configuration with two items on one face's
   protocol, which is outside the plan's shape; zk2py did not measure what
   zenoh does with one.
3. **§2.3 and Appendix D, two revisions of one interface disagreeing on
   `link.exposure`.** The text does not decide: F-119.
4. **§2.11, "a window not given is 60 s", beside the table's "not asked: no
   windows were given".** Two absences, both decided. The table's is the
   linter given no windows at all: the question is not put. §2.11's is a
   windows map given that names no window for a service: that service has
   core S3's 60 s. `lints.json` agrees: its description ("windows, present
   only with implements … an address it does not name has the core's 60
   s"), cases 53 and 54 with no `windows` and no window finding, and case
   56 with `windows = {}` and `window_under_outage`. zk2py reads it so, and
   files nothing.

### F-118 · ambiguity · link.v1 0.2 §2.4: a near principal on the face

> §2.4: "In zenoh 1.10.1, a session matches every subject whose properties
> all match it, and across those subjects any allow wins" … "A generator
> MUST therefore put every deny of a face's plan into the policy of each
> subject that can match a session on the face: the far principal's, at
> least." … "The near side's principals connect elsewhere, so their
> policies carry none of it."

> §2.4: "A session on the face is a principal's" … "A generator MAY also
> narrow the far principal's subject to the face's link protocols."

A subject selected by a user name or a CN alone matches that principal's
session on whatever link it arrives, so a near principal's subject "can
match a session on the face": its credentials are accepted there. The text
then puts none of the denies in its policy, because the near principals
"connect elsewhere": a fact of the deployment the generator cannot see, and
the router does not enforce. Measured in scenarios.md §1's deployment: a
session with `local`'s credentials over the `unixsock-stream` face sees the
owner's instance and interface tokens and receives every push, 100 of
`air/frames_total` and of `raw`, all 5 `change` occurrences, while `ground`
on the same face receives none of them. The text does not say whether a
generator narrows each near principal's subject to its own transports (the
counterpart of §2.4's MAY for the far principal), puts the denies in every
policy whose subject is not so narrowed, or leaves it to the deployment's
credentials.
**Resolved:** a guess. zk2py puts the denies in the far principal's policy
alone, as the scenarios' block does, and records the measurement.

**Status at link.v1 0.3: resolved by 0.3, zk2py's guess kept and completed.** §2.4: "A session over a face that is not the far principal's MUST get nothing across the face that the plan denies, and never more than the far principal gets there." And: "A subject bound to a principal is selected by the link protocols of §2.1's closed list outside every face whose far principal it is not, in that list's order." "Under `deny`, that is all: a session that matches no subject is granted nothing (core §11.3)." Under `allow`, "Each face therefore has a subject of its own, selected by its link protocols alone and bound to no principal, which the far principal's policy lists beside the far principal's own subject." §2.15 adds `near_subject_on_face` and `face_subject_missing`. zk2py's generator is now `link.compile_faces`; scenarios §7 step 2 is this entry's measurement, and §1's measurement row is gone.

### F-119 · gap · link.v1 0.2 §2.2, §2.3, §2.7, with core §9.8: two revisions of one interface

> §2.3: "A generator derives a plan for each face of a policy, from the
> policy, the interfaces each listed service implements (the deployment's,
> as core §11.1's input names them), and the contracts of those
> interfaces."

> §2.2: "Exposure is the contract's, for every deployment."

> core §9.8: "| `cardinality`, `epoch`, `gate` or `annotations` changed |
> review |"

An annotation changed between two revisions is a review, not a breaking
change, so one major's minors may differ in a resource's `link.exposure`,
and two services of one deployment may run different minors (each
instance's descriptor states the minor it runs, core §3.3). The plan's exposure denies
are per interface id, `zk2/*/*/I/K/…`, the same keys for every service that
implements `I`, while its downsampling rules and liveness evidence are per
service. When `radio-a` runs a minor that marks `air/frames_total` `link`
and `radio-b` one that leaves it `host`, the text names no revision to
derive from: the newer's carve-out lets `radio-b`'s `host` stream cross,
the older's denies `radio-a`'s exposed one. §2.7's "What a later minor
adds" covers a resource added, not an exposure changed, and Appendix D
publishes the key, not a rule across revisions. `plan.json`'s deployment
gives one resource list per interface id, so no case has two.
**Resolved:** a guess. zk2py derives a plan from one revision per interface
id, as `plan.json` gives it. Were two given, it would let a resource cross
only where every revision present marks it `link`, failing closed as §2.2
fails closed on an invalid value.

**Status at link.v1 0.3: resolved by 0.3, as zk2py guessed, with two rules more.** §2.3: "The plan reads each listed interface once, over every revision of it the generator is given." "A resource is exposed only where every revision that declares it reads `link` (§2.2). Where one reads `link` and another does not (`host`, absent, or no exposure at all), it is read as `host`, and a face that exposes reports it (`exposure_disagrees`, below)." "A resource has a horizon (§2.12) only where every revision that declares it gives one above 0; its horizon is then the least of them." The horizons and the warning are new to zk2py. `plan.json`'s fixture format takes `{revisions: [resources, ...]}`, and zk2py reads two `contracts` of one interface id as two revisions (`link.merge_revisions`). Scenarios §8 runs it.

**Where zk2py's reading of link.v1 0.2 needed a guess, beyond the
findings.**
- **The enrollment's format** is the reference generator's own (core §11.1,
  "The input"): zk2py reads `radio.enrollment.toml` as its comments say, a
  binding without `resources` taking every stream, state and event resource
  of its interface.
- **§2.5's grant for a state GET** names both `state/**` and `@state/**` at
  each provider as bound whenever a binding names a state resource.

## At link.v1 0.3 (#609)

`link.v1` 0.3 answers F-118 and F-119. For F-118, every other principal's
subject is narrowed off a face, and under `allow` the face has a subject
of its own in the far principal's policy; two §2.15 checks judge a block's
subjects. For F-119, every revision given is read together, failing
closed, with the warning `exposure_disagrees`. It adds scenarios §7 and §8,
and changes a comment of `examples/zk2/link/radio.enrollment.toml`. Core
stays at 0.30. zk2py read it cold, and four questions.

**`just py-conformance`: 1386 of 1386,** with no unknown file. The `link`
family is 128: `exposure.json` 31, `plan.json` 12 (one new, two revisions)
and `lints.json` 85 (eight new: two with revisions, six with a block's
subjects and posture). Every new case passed the first time it ran. Each
mutant of the new rules fails at least one case:
- no subject check, four;
- the far principal's subject not exempted, six;
- CNs ignored, or interfaces ignored, one each;
- one revision read (the first or the last), or any `link` winning, two;
- the greatest horizon in place of the least, one.

**`just py-link`: 41 of 41,** all eight sections, with `NOTE` lines, not
counted, for what the text states no expectation for. §1 to §6 run on
0.3's generator, every subject but the far principal's narrowed off the
face (§1 now checks it), and pass as before.

**§7**, X being `local`'s credentials over the face. "§1's share" is 5 of
`air/dropped_total` and of `frames`, none of `air/frames_total` or `raw`,
all 10 `oper` puts, the 5 `fault` occurrences and no `change`. "`local`'s
share" is 100 of `air/frames_total` and of `raw`, the 5 `change`
occurrences, the owner's two tokens and its descriptor, and still 5 of
`air/dropped_total` and of `frames` (the face's downsampling).

| Step | Posture, block | G | X |
|---|---|---|---|
| 1 | `deny`, 0.3's | §1's share, no token or descriptor, its GET and call answered; its puts do not reach L | nothing at all: no push, token, descriptor or reply; its puts do not reach L |
| 2, control | `deny`, 0.2's | §1's share | `local`'s share, its GET and call answered; its puts do not reach L |
| 3 | `allow`, 0.3's (`face:radio` in `ground`'s policy) | §1's share, no token, its GET and call answered; its puts do not reach L | the same as G |
| 4, control | `allow`, narrowed, no face subject | §1's share | `local`'s share, its GET and call answered, and its 5 puts reach L |
| 5, control | `allow`, `face:radio` in a policy of its own | §1's share, its GET and call answered, and its 5 puts reach L | the same as G |
| 2 under `allow` | `allow`, 0.2's | §1's share | `local`'s share; its puts do not reach L |
| 5 under `deny` | `deny`, `face:radio` in a policy of its own | §1's share; its puts do not reach L | nothing at all |

- **Both postures.** Steps 1 and 3 are each other's, and step 4's block
  under `deny` is step 1's byte for byte (no face subject under `deny`), so
  it was not run twice. Steps 2 and 5 were also run under the other
  posture (the last two rows).
- **L**, in step 1: 100 of each stream, 10 `oper`, 5 `fault` and 5
  `change`.
- **Step 5's control does what the text says.** G's 5 puts on
  `lab/radio-a`'s `air/dropped_total` reach L, where under step 3's block
  none do. The face subject's own policy allows what its denies do not
  include, and any allow wins across subjects, so `ground`'s deny of other
  services' writes is undone.
- **Step 6.**
  - Steps 1 and 3: no finding.
  - Step 2: `near_subject_on_face` for `ground-router`, `local`, `radio-a`
    and `radio-b`.
  - Step 4: `face_subject_missing` at `unixsock-stream`.
  - Step 5: no §2.15 finding. zk2py's comparison with the plan
    (`link.compare_block`, a shape of its own) names `face:radio-policy`.
  - 0.2's block under `allow` gets both checks' findings.
- **A session with no credential at all** over the face, under step 3's
  block: N refuses it at the link (usrpwd), as core §11.3 requires under
  `allow`.

**§8.** Both owners publish `air/frames_total` and `air/dropped_total` at
20 Hz for 5 s. `lab/radio-a` runs minor 1, written beside the run from the
example with `minor = 1` and the annotation, and `lab/radio-b` runs minor 0.
- **Both revisions given:** the plan warns `exposure_disagrees` at
  `radio.v1/stream/air/frames_total`, and its denies and rules are minor
  0's alone. G receives none of `air/frames_total` from either owner and 5
  of `air/dropped_total` from each. L receives 100 of `air/frames_total`
  from each.
- **Minor 1 alone:** a rule for each owner's `air/frames_total`, and no
  deny of it. G receives 5 of `air/frames_total` from each owner,
  `lab/radio-b`'s among them though minor 0 keeps it home, and 5 of
  `air/dropped_total` from each.

**What changed in zk2py.**
- **`zk2py.link`:**
  - `merge_revisions` reads every revision given, and the plan warns
    `exposure_disagrees`;
  - `check_subjects` holds the two checks, which `check_block` runs when
    the block carries its subjects and posture;
  - `compile_faces` is 0.3's generator: the denies, the narrowing, the
    face's subject. `compile_face` stays as 0.2's, for §7's control;
  - `compare_block` is zk2py's comparison of a block with the plan.
- **`zk2py.link_scenarios`:** §7 and §8, every section on 0.3's
  generator, and §1's F-118 measurement removed (§7's step 2 is it).
- **`zk2py.conformance`:** two `contracts` of one interface id are two
  revisions of it.

**The four questions, read from the text alone.**
1. **§2.4 under `allow`, a session over the face with no credential at
   all.** It gets the far principal's share, from the face's own subject
   (`face:radio`), which the far principal's policy lists. §2.4: "A session
   over the face matches it, and gets what the far principal gets there,
   never more". It matches no other subject: each is bound by a user name,
   and the far principal's, the only one left on the face, needs its user
   name. Under `deny` it matches no subject and gets nothing. Core §11.3
   also requires that "A deployment under `allow` MUST refuse
   unauthenticated sessions at the link", and N does (measured above), so
   the face subject is a second guard. Decided.
2. **§2.15, a subject whose user names include both the far principal and
   a near one.** Not reported: "none of them the face's far principal" is
   false when one of them is. That is decided. But such a subject matches
   the near principal's session over the face, and what that session gets
   is whatever the policies listing the subject grant, which §2.15 says it
   cannot see. Measured: listed in `local`'s policy alone, it gives X
   `local`'s share over the face, and the tool reports nothing. That is
   F-120 (a), with two more blocks of the same kind.
3. **§2.3, "every revision that declares it", for a resource the older
   minor does not declare.** It is read from the revisions that declare
   it, here the newer alone: "A revision that does not declare a resource
   says nothing of it: a resource a later minor adds is read from the
   revisions that declare it." Decided.
   - It crosses where the newer exposes it (`plan.json`'s `air/snr`), for
     every listed service of the interface, including the older minor's.
     That service never publishes it: its rule names a key with no
     traffic, and its liveness evidence a member that never turns fresh,
     so at worst unobservable, never down (core R7).
   - §2.3 pairs resources "by kind token and template", and core §9.8 by
     kind and template. An `explicit` cleared between minors
     (`explicit_cleared`, a review) moves a resource from `@stream` to
     `stream`, which link.v1 reads as two resources, each from the revision
     that declares it. Their keys differ, so that is right for a router.
     No finding.
4. **§2.4, the block's subjects for §7's deployment** (from
   `radio.enrollment.toml` and `radio.face.toml`):
   - `radio-a`, `radio-b`, `local` and `ground-router`, each bound by its
     user name and selected by `["tcp", "udp", "tls", "quic", "serial",
     "unixpipe", "vsock", "ws"]`: §2.1's list without `unixsock-stream`, in
     its order. `ground-router` is narrowed too: it is a principal of the
     block, though another policy's far principal (§2);
   - `ground`, bound by its user name, with no `link_protocols`: it is
     narrowed off no face. Under `deny` §2.4 lets a generator narrow it to
     `unixsock-stream`. Under `allow` it MUST NOT;
   - under `allow`, one more subject, selected by `["unixsock-stream"]` and
     bound to no user name, CN, interface or zid, which `ground`'s policy
     lists beside `ground`'s subject. Under `deny`, none.

   What the text leaves to the generator:
   - every subject's id, the face subject's included: `face:radio` comes
     from §7 and Appendix B, not from a rule;
   - the policies' ids, and the order of the subjects in the block and in
     `ground`'s policy;
   - whether `ground`'s subject is narrowed under `deny`. zk2py does not
     narrow it.

   §7's expected 1 and 3 fix the protocols and the face subject's place,
   and zk2py's generator writes exactly those.

### F-120 · gap · link.v1 0.3 §2.15, with §2.4: blocks the subject checks pass that §2.4 forbids

> §2.15: "`near_subject_on_face` (error, `at` the subject's id): a subject
> bound by a user name or a CN, none of them the face's far principal,
> whose `link_protocols` are absent or name one of the face's, whatever
> else it names" … "`face_subject_missing` (error, `at` the link protocol):
> under `allow`, a link protocol of the face that no subject bound by no
> user name, CN, interface or zid names in its `link_protocols`"

> §2.4: "A session over a face that is not the far principal's MUST get
> nothing across the face that the plan denies, and never more than the
> far principal gets there." … "Each face therefore has a subject of its
> own, selected by its link protocols alone and bound to no principal" …
> "Under `allow` it MUST NOT be narrowed to its own faces' link protocols:
> outside them, its session would match no subject, and be allowed
> everything."

> §2.15: "Whether a block holds the whole plan, every deny and every rule,
> and whether a face's own subject rides the far principal's policy, is a
> comparison of the block with the plan" … "§2.15 cannot tell which of a
> policy's denies are the far principal's own."

The two checks pass three blocks that §2.4 forbids. Each is a hand edit of
zk2py's 0.3 block for §7's deployment, measured on N (the `NOTE` lines of
`just py-link`'s §7):
- **(a) A subject bound to the far principal and a near one.** Under
  `deny`, `local`'s subject is replaced by one bound to `ground` and
  `local` both, listed in `local`'s policy. X, `local`'s credentials over
  the face, receives `local`'s share there: 100 of `air/frames_total` and
  of `raw`, the 5 `change` occurrences and both tokens. That is more than
  the far principal gets. `near_subject_on_face` exempts the subject,
  since one of its names is the far principal's. G kept §1's share.
- **(b) A face subject that also names a near protocol.** Under `allow`,
  the face's own subject names `tcp` beside `unixsock-stream`. It then
  matches the near side's sessions, which get the far principal's grants
  beside their own, since any allow wins. A session with `radio-a`'s
  credentials over TCP reads `lab/radio-b`'s `air/dropped_total`, 20 of 20
  samples, where 0.3's block allows it none. `face_subject_missing` is
  satisfied, since the subject names the face's protocol. Yet §2.15 says a
  subject with no property is not the face's own because "it matches
  every session, the near side's included (core §11.2, F-87)".
- **(c) The far principal narrowed to its face.** Under `allow`,
  `ground`'s subject is narrowed to `unixsock-stream`. `ground`'s
  credentials over TCP then match no subject, and its 5 puts on
  `lab/radio-a`'s key reach L (none under 0.3's block). Neither check
  looks at the far principal's subject.

The tool reports no finding on any of the three. 0.3's generator writes
none of them, but a router can run each, and §0 says "checks read what a
router runs". The text gives two other properties to the comparison with
the plan, and says nothing of these three. Each can be decided from the
block, the policy and the posture alone: a subject's names and link
protocols.
**Resolved:** open. zk2py's checks report exactly what §2.15 lists, and the
three blocks pass them. A reading that would catch them:
- `near_subject_on_face` for a subject bound to any principal other than
  the far one, whatever else it names;
- under `allow`, a finding for a subject bound to no principal that names
  a link protocol outside the face;
- under `allow`, a finding for the far principal's subject whose link
  protocols are all the face's.

**Status at link.v1 0.4: resolved by 0.4, every block reported as zk2py measured it.** §2.15: "`near_subject_on_face` (`at` the subject's id): a subject on the face bound to another principal, whatever else it names, the far principal's name among them (0.4)." "`face_subject_beyond_face` (`at` the subject's id): a face subject whose `link_protocols` are absent, or name one outside the face's". "`narrowed_too_far` (no face, `at` the user name or CN): under `allow`, a name the block's subjects bind, or a face's `far`, and a link protocol of §2.1's list outside every face whose far principal it is not, that no subject naming no interface and no zid selects, among those that name it or no user name and no CN (0.4)." And for two more blocks of the same kind, a face subject in a near principal's policy or in none: "`face_subject_misplaced` (`at` the subject's id): a face subject not listed in exactly the far principal's policies, at least one (0.4)." §2.15 also explains why G kept §1's share in (a): "equal combinations, across subjects, are one subject, whose rules are their policies' together, deny first". zk2py's §9 step 2 confirms it on zenoh-python: with `ground`'s subject narrowed, G gets `local`'s share. `lints.json`'s 18 new cases pass, and scenarios §9 and §10 run each block.

**Where zk2py's reading of link.v1 0.3 needed a guess, beyond the
findings.**
- **The face subject's id** is `face:<face>`, after §7 and Appendix B. The
  far principal's subject is not narrowed under `deny`: zk2py does not
  take the MAY.
- **A list given empty** in a block's subject (`link_protocols: []`) reads
  as absent. `lints.json`'s description writes "each list absent when the
  subject names none", and no case has an empty list.
- **Two refusals have no code.** zk2py's generator refuses a face whose
  far principal has no subject in the block (`link.Refused`). It also
  refuses a principal left with no link protocol, which the text says a
  generator refuses.
- **The comparison with the plan** (`link.compare_block`) has no shape in
  the text. zk2py's lists, per face:
  - a deny of the plan missing from the far principal's policy;
  - under `allow`, a face whose own subject does not ride that policy,
    naming the policies that list it instead.
- **A subject bound to the far principal and a near one** never comes
  from zk2py's generator, which writes one subject per principal. Were it
  given one, it would not narrow it, since the subject names the far
  principal.
- **Revisions.** A resource no revision exposes keeps the first declaring
  revision's reading, since host, invalid and ignored are alike to the
  plan. One revision is read as itself.

**Status at link.v1 0.4:** 0.4 settles four of these guesses (the
changelog's "The guesses round 32 listed, settled"):
- every subject id is the generator's: "no check depends on what it says";
- an empty list "is refused when the configuration loads, so no router
  runs one. A generator never writes one, and a check given one reads it as
  absent", as zk2py read it. zk2py's §9 step 6 measures the refusal;
- the generator's refusals are its own, "this text gives them no code";
- the comparison's shape is "the generator's".

## At link.v1 0.4 (#609)

`link.v1` 0.4 answers F-120. §2.15 now reads which policies list each
subject, defines a subject on the face, the far principal's, a face
subject and the far principal's policies, and reports every block of
F-120's kind:
- `near_subject_on_face` is widened;
- three checks are new: `face_subject_beyond_face`,
  `face_subject_misplaced` and `narrowed_too_far` (no face);
- it states the denies a face's block holds, each subject that can match
  the far principal's session checked alone.

It records three zenoh facts: the split into combinations, a subject no
policy lists, and an empty list refused at load. It settles round 32's
guesses and adds scenarios §9 and §10. Core stays at 0.30. zk2py read it
cold, and three questions.

**`just py-conformance`: 1404 of 1404,** with no unknown file. The `link`
family is 146: `exposure.json` 31, `plan.json` 12, `lints.json` 103 (18
new). Every case passed the first time it ran.

**Mutating the new rules** (`lints.json`), 15 mutants. Ten fail at least
one case:
- `near_subject_on_face` not widened, 2;
- "the far principal's" read as the far principal among the names, 2, or
  with CNs ignored, 2;
- `face_subject_beyond_face` for absent link protocols only, 1, or for
  protocols outside the face only, 2;
- `face_subject_missing` answered by a face subject naming an interface,
  1;
- absent link protocols read as off the face, 11;
- `narrowed_too_far` ignoring a face's `far`, 1; requiring every link
  protocol, 1; under `deny` too, 2.

Five survive, rules no case pins:
- `face_subject_misplaced` comparing "contains" for "exactly": no case
  lists a face subject in every policy of the far principal's and in one
  more;
- `face_subject_misplaced` without "at least one": no case has a face
  subject in no policy while the far principal has none either;
- `narrowed_too_far` ignoring the subjects that name no user name and no
  CN: no case has one answering a link protocol the name's own subjects do
  not;
- `narrowed_too_far` counting subjects that name an interface or a zid: no
  case has one under `allow`;
- an empty list read as empty, not absent: "No list is ever empty" in the
  fixture, by design.

**`just py-link`: 63 of 63,** all ten sections, with `NOTE` lines for
measurements the text states no expectation for. §1 to §8 pass as before,
and §7's step 6 now reports `face_subject_misplaced` for step 5. The tool
is `link.check_router_block`, which reads N's whole block, each subject
that can match the far principal's session checked alone.

**§9**, X being `local`'s credentials over the face:
- **Step 1** (`deny`, `local`'s subject bound to `ground` and `local`, in
  `local`'s policy). X gets `local`'s share: 100 of `air/frames_total` and
  of `raw`, the 5 `change` occurrences, both tokens and the descriptor,
  its GET and call answered, and 5 of `air/dropped_total` and of `frames`.
  G gets §1's share. Neither's puts reach L.
- **Step 2** (the same, `ground`'s subject narrowed to `unixsock-stream`).
  G gets `local`'s share too, exactly as X does, and neither's puts reach
  L. The combination-merging explanation holds on zenoh-python: round 32's
  G kept §1's share only because the combined subject's `ground` part was
  `ground`'s own subject.
- **Steps 3 and 4** (`allow`): `ground` narrowed to `unixsock-stream`, and
  `radio-b` narrowed off `tcp` too. Each one's session over TCP puts 5
  times, and all 5 reach L.
- **Step 5** (the generator's `allow` block): 0 of either's.
- **Step 6:** zenoh-python refuses both empty lists, `link_protocols: []`
  and `usernames: []`, at `Config.insert_json5` ("Given collection was
  empty"), before any session opens. The same subject without the list
  loads.
- **Step 7:**
  - steps 1 and 2: `near_subject_on_face` at `local`, and `zk_not_denied`;
  - step 3: `narrowed_too_far` at `ground`, and step 4 at `radio-b`, each
    with no face;
  - the generator's `allow` block with `ground`'s subject removed:
    `narrowed_too_far` at `ground` and `face_subject_misplaced` at
    `face:radio`;
  - step 5's block: no finding.

**§10**:
- **Step 1**, under each posture: a face subject selected by `tcp` beside
  `unixsock-stream`, in `ground`'s policy. The `radio-a` session over TCP
  receives all 20 of `lab/radio-b`'s samples, and so does L.
- **Step 2** (the generator's blocks): it receives none under either
  posture, and L all 20.
- **Step 3** (`deny`, a face subject in `local`'s policy) and **step 4**
  (`allow`, the face's subject moved to `local`'s policy): G and X each get
  `local`'s share, and neither's puts reach L.
- **Step 5** (`allow`, the face's subject in no policy): G and X each get
  `local`'s share, and all 5 of each one's puts reach L.
- **Step 6** (`deny`, the face's subject in a policy of its own holding
  the plan's denies): G gets §1's share and X nothing at all, with no token
  and no reply. Neither's puts reach L.
- **Step 7:**
  - step 1, each posture: `face_subject_beyond_face` at `face:radio`;
  - steps 3 to 5: `face_subject_misplaced` at `face:radio`, and
    `zk_not_denied`;
  - step 6: `face_subject_misplaced` alone;
  - the control blocks: no finding.

**What changed in zk2py.**
- **`zk2py.link`:**
  - `check_subjects` reads the policies' subject lists, with `on_face`,
    `far_principals`, `is_face_subject` and `far_policies` as §2.15
    defines them;
  - `narrowed_too_far` reads every face;
  - `subject_denies` and `check_router_block` read a router's whole block,
    each subject's denies alone;
  - `compare_block` keeps the denies, its shape zk2py's.
- **`zk2py.link_scenarios`:** §9 and §10, §7's step 6 for 0.4, and F-121's
  measurements. Round 32's F-120 measurements became §9's and §10's steps.

**The three questions, read from the text alone.**
1. **`narrowed_too_far`: which link protocols a name must be selected by.**
   The check names "a link protocol of §2.1's list outside every face
   whose far principal it is not". In §7's deployment there is one face, `radio`, with far
   principal `ground`, over `unixsock-stream`.
   - **`radio-b`** is not its far principal, so it must be selected over
     the other eight: `tcp`, `udp`, `tls`, `quic`, `serial`, `unixpipe`,
     `vsock` and `ws`. The generator's subject for `radio-b` names exactly
     those, and §9's step 4 removes `tcp`.
   - **`ground`** is the far principal of every face, so it is kept off
     none and must be selected over all nine. Over `unixsock-stream`, the
     face's subject answers, since it names no user name and no CN. Over
     the eight others, only `ground`'s own subject answers, unless another
     subject names no user name and no CN.
   - Decided. One note: a subject that names no user name and no CN and
     selects near protocols alone answers every name over them. Such a
     subject is off the face, so no check of 0.4 reports it, and the
     changelog leaves it to core §11.2 (F-87).
2. **A subject naming `usernames = ["ground"]` and `cert_common_names =
   ["local"]`.**
   - **§2.15 decides that it is the far principal's alone.** Its
     user-name list is not empty and every name in it is `ground`, and "A
     subject bound to another principal names a user name or a CN, and is
     not the far principal's". So it is not both. `lints.json`'s case 86
     pins this: its subject with user name `local` and CN `ground` is the
     far principal's and draws no finding.
   - **zenoh matches it only for a session presenting the user name
     `ground` and the CN `local`.** "zenoh 1.10.1 matches one value of each
     property a subject names". Measured with usrpwd: listed in `local`'s
     policy under `deny`, it gives G (user name `ground`, no certificate)
     §1's share and nothing of `local`'s, and X nothing.
   - **Two other passages read it otherwise.** §2.4's "A subject that names
     the far principal and another principal binds the other too … the
     other's session over the face matches it, whatever else it names" is
     true of names in one property, not across the two. And
     `narrowed_too_far` credits the subject to `local`, as its CN. That is
     F-121.
3. **`face_subject_misplaced` when the far principal has no subject on
   the face at all.** Then the far principal's policies are none, and
   every face subject is misplaced, wherever it is listed: "not listed in
   exactly the far principal's policies, at least one". `lints.json`'s
   cases 101 (`ground`'s subject in no policy) and 102 (no subject for
   `ground`) pin it, as does §9's block with `ground`'s subject removed.
   Under `deny` with no face subject, nothing is reported, and the far
   principal gets nothing across the face, failing closed. Decided.

### F-121 · gap · link.v1 0.4 §2.15 `narrowed_too_far`, with §2.4: a name beside, or as, the other kind

> §2.15: "`narrowed_too_far` (no face, `at` the user name or CN): under
> `allow`, a name the block's subjects bind, or a face's `far`, and a link
> protocol of §2.1's list outside every face whose far principal it is
> not, that no subject naming no interface and no zid selects, among those
> that name it or no user name and no CN (0.4). A session with that name
> over that link protocol matches no subject, and is allowed everything
> (core §11.3): the far principal narrowed to its own faces (§2.4's MUST
> NOT), or left with no subject at all, or any principal narrowed further
> than §2.4 narrows it."

> §2.15: "A subject is the far principal's when it names the far principal
> alone, as its user names or as its CNs: one of the two lists is not
> empty, and every name in it is the face's `far`. zenoh 1.10.1 matches
> one value of each property a subject names, so every session such a
> subject matches presents the far principal's name."

> §2.4: "A subject that names the far principal and another principal
> binds the other too: zenoh 1.10.1 matches one value of each property a
> subject names, so the other's session over the face matches it,
> whatever else it names, and gets the grants of every policy listing it."

The check's "those that name it" counts a subject that names the name
anywhere in its user names or CNs. By the zenoh fact §2.15 itself states, such a subject
matches a session with that name only when the session presents every
other property the subject names. Two blocks therefore pass the check
while a session with the name matches no subject. Both are hand edits of
the generator's `allow` block for §7's deployment, with `local`'s subject
narrowed off `tcp` too. Measured on N (`NOTE` lines of `just py-link`'s
§9); every session authenticates by user name and password:
- **(i) the name beside a name of the other kind.** A subject
  `usernames = ["ground"]`, `cert_common_names = ["local"]`, in `ground`'s
  policy. `narrowed_too_far` credits it to `local` over `tcp` and reports
  nothing. `local`'s session over TCP, with no certificate, matches no
  subject: its 5 puts on `lab/radio-a`'s key reach L. Without the subject,
  the tool reports `narrowed_too_far` at `local`, for the same 5 puts.
- **(ii) the name as the other kind.** A subject `cert_common_names =
  ["local"]`, selected by the eight near link protocols and so off the
  face, in `local`'s policy. The check credits it to `local` and reports
  nothing. `local`'s session, presenting the user name `local`, matches
  neither subject: its 5 puts reach L.

§2.4's "binds the other too … whatever else it names" reads names the
same way. For names in one property it is right, since one value of that
property is matched. Across the two properties it is not: a session with
the other principal's name alone does not match. §2.15's definition of
the far principal's subject, and `lints.json`'s case 86, decide that a
subject naming `ground` as its user name and another principal as its CN
is the far principal's. So the checks read it consistently, and only
`narrowed_too_far` credits a name a session does not satisfy.
**Resolved:** open. zk2py's `narrowed_too_far` reports exactly what
§2.15 says, and passes both blocks. A reading that would catch them: a
subject answers for a name only where it names that name in one list,
user names or CNs, and names no other list; and a name the block binds as
a user name is answered among user names, a CN among CNs. A face's `far`
that no subject binds would take either.

**Status at link.v1 0.5: resolved by 0.5, as zk2py measured it and read it.** §2.15: "A subject is the far principal's when it names the far principal alone in one of its two lists and nothing in the other: its user names are all the face's `far` and it names no CN, or its CNs are all the face's `far` and it names no user name (0.5)." "A subject bound to another principal names a user name or a CN, and is not the far principal's: a subject naming both kinds is one." For `narrowed_too_far`: "A subject that names no interface and no zid answers for a user name when it names that user name among its user names and no CN, or names no user name and no CN; for a CN, the same with the two lists swapped. A face's `far` that no subject names is answered only by a subject that names no user name, CN, interface or zid." §2.4 now says the generator's side: "Across its two lists, a subject binds no one alone". zk2py's reading, "a name the block binds as a user name is answered among user names, a CN among CNs", is the rule. 0.5 also closes a hole zk2py's question 2 had answered the other way: a subject naming the far principal beside a CN of another principal no longer makes that policy the far principal's (`lints.json` cases 105, 106). zk2py's scenarios §11 runs both blocks, and its mTLS step confirms the zenoh fact: a session presenting the CN matched the subject naming both kinds, and one presenting another CN, or none, did not.

**Where zk2py's reading of link.v1 0.4 needed a guess, beyond the
findings.**
- **One `zk_not_denied` for the whole block.** zk2py's tool reports a
  single `zk_not_denied` (`at` `@zk`) when the denies of any subject that
  can match the far principal's session, or of none, miss a pair.
  `lints.json` gives one subject's denies, so it pins no count.
- **One `narrowed_too_far` per name**, whichever link protocols are
  missed, since `at` is the name.
- **A block with one or two of `default_permission`, `subjects` and
  `policies`** has its subjects checked by none of the subject checks: the
  fixture gives all three or none.
- **Policies are told apart by position:** the fixture's carry no id.

## At link.v1 0.5 (#609)

`link.v1` 0.5 answers F-121.
- The far principal's subject names it alone in one list and nothing in
  the other.
- A subject naming both kinds is bound to another principal, and gives
  the far principal no policy.
- `narrowed_too_far` reads names by kind.
- A generator on a router with a face refuses a principal bound both ways.

0.5 records two more zenoh facts: a subject naming both kinds matches only
a session presenting both, and a subject two policies list holds both's
rules. It adds scenarios §11 and ten `lints.json` cases, and changes case
86. Core stays at 0.30. zk2py read it cold, with two questions, and fixed
what review-bot found in round 33's code (PR #787).

**`just py-conformance`: 1414 of 1414,** with no unknown file. The `link`
family is 156: `exposure.json` 31, `plan.json` 12, `lints.json` 113. Every
case passed the first time it ran on 0.5's rules.

**Mutating the rules** (`lints.json`), 15 mutants. Eleven fail at least
one case:
- the far principal's subject allowed to name the other list too, 4 (and
  only the CN side allowed to, 2);
- `narrowed_too_far` crediting a subject naming the name as the other
  kind, 1;
- `narrowed_too_far` crediting a subject naming both kinds, 1 on the
  user-name side and 1 on the CN side;
- an untold `far` answered by any named subject, or not read at all, 1
  each;
- round 33's four survivors, all now pinned:
  - `face_subject_misplaced` comparing "contains", 3;
  - `face_subject_misplaced` without "at least one", 1;
  - no-name subjects not answering, 1;
  - interface or zid subjects answering, 1.

Four survive:
- **a name in both lists checked as one kind only** (its first): no case
  has a name whose first kind passes and whose second fails;
- **one finding per kind**: equivalent, since findings are a set (§2.1)
  and `at` is the name;
- **an empty list read as empty**: unpinned by design (0.5: "stays
  unpinned");
- **a rule without `flows` read as applying to none**: `lints.json`'s
  `deny` always carries flows, so only the router reading pins it.

**`just py-link`: 78 of 78,** all eleven sections. §1 to §10 pass as in
round 33, and §7 again measures step 2's block under `allow` (a `NOTE`).
§11:
- **Step 1** (F-121 (i), `allow`): `local`'s session over TCP, 5 of 5
  puts reach L.
- **Step 2** (F-121 (ii)): 5 of 5.
- **Step 3** (control): `local` narrowed off `tcp` alone, 5 of 5. The
  generator's block, 0 of 5.
- **Step 4**, mTLS, on zenoh-python. N listens on `tls/127.0.0.1` with
  `enable_mtls`, its certificate and the clients' signed by a CA openssl
  mints for the run (P-256). `local`'s subject names `local` as its user
  name and as its CN:
  - over TLS presenting the CN `local`: 0 of 5;
  - over TLS presenting the CN `other`: 5 of 5;
  - over TCP with no certificate: 5 of 5.

  Under the generator's block, 0 of 5 each. A session presenting the CN
  matches the subject naming both kinds; one presenting another CN, or
  none, does not.
- **Step 5** (`deny`, `ground` and `radio-a`'s CN in `radio-a`'s policy,
  the face's subject in `ground`'s and `radio-a`'s): G and X each get §1's
  share, and all 5 of each one's puts reach L.
- **Step 6** (the same with `local`): §1's share each, and no puts reach L.
- **Step 7** (step 5 under `allow`): G gets §1's share. X gets nothing at
  all: no push, no reply, no token. Neither's puts reach L. That is
  F-122.
- **Step 8** (step 5, `radio-a`'s policy also listing the face's `zk`
  rule): all 5 of each one's puts reach L.
- **Step 9, the tool:**
  - step 1: `narrowed_too_far` at `local`, and `near_subject_on_face` at
    `ground-local`;
  - steps 2 to 4: `narrowed_too_far` at `local`;
  - steps 5 and 6: `near_subject_on_face` at the added subject,
    `face_subject_misplaced` at `face:radio`, and `zk_not_denied`;
  - step 7: the same, and `narrowed_too_far` at `radio-a`;
  - step 8: `near_subject_on_face` and `face_subject_misplaced`;
  - the generator's blocks: no finding.

**Robustness** (review-bot on PR #787):
- **A block zenoh refuses at load is an error.** Measured, zenoh 1.10.1
  refuses:
  - a subject with no id ("missing field `id`");
  - two subjects with one id;
  - a policy listing a subject, or a rule, the block does not define;
  - a policy with no rules;
  - a rule with no `messages`.

  zk2py's checks now raise `link.BlockError` for the first three, naming
  the subject, policy or rule, where a `KeyError` or a silent collision
  had been. `link_scenarios` checks every hand-edited block the same way
  before a router loads it.
- **An unknown face** given to `check_router_block` raises `BlockError`
  naming the faces the policy has, not `StopIteration`.
- **A rule without `flows` applies to both flows** (measured on a router):
  - under `deny`, every allow without flows admitted a put end to end, 5
    of 5;
  - with the publisher's allow on egress alone, 0 of 5; on ingress alone,
    5 of 5;
  - with the subscriber's allow on one flow alone, 0 of 5;
  - under `allow`, a deny without flows refused a put on ingress (for the
    publisher) and on egress (for the subscriber), 0 of 5 each;
  - the subscriber's deny on egress alone refused it, 0 of 5; on ingress
    alone it did not, 5 of 5.

  `link.rule_flows` reads a missing `flows` as both, and every other key
  of a rule is read with `.get`.
- **`downsampling`** defaults to `None` (a `Sequence`) in
  `check_router_block` and `link_scenarios.check`.
- **§7's coverage.** Every step §7's text names runs: steps 1 to 5 as five
  blocks, and step 6's tool on each. Round 33 dropped only round 32's own
  additions: step 2's block under `allow`, which is back as a `NOTE`; step
  5's block under `deny`, which is §10's step 6; and the F-120 probes,
  which are §9's and §10's steps. No step lost coverage.

**The two questions, read from the text alone.**
1. **A name that one subject binds as a user name and another as a CN.**
   It is "checked as each".
   - As a user name it is answered by the subjects naming it among their
     user names and no CN, or naming neither kind. As a CN it is answered
     by those naming it among their CNs and no user name, or neither kind.
   - Each kind is checked over every link protocol outside the faces whose
     far principal it is not.
   - It gets one finding at most, at the name: "It reports a name once,
     whichever of its kinds fails". (Findings being a set, §2.1, that
     holds anyway.)
   - Case 111, one subject naming `local` in both lists, answers for
     neither kind. Only the face subject, over `unixsock-stream`, is left,
     where `local` is kept off anyway, so `local` gets one finding.
   - Decided. One observation, no finding: the CN kind is checked over
     every protocol, though zenoh takes a CN only from a TLS or QUIC
     link's certificate. Case 112 reports the CN `ops` narrowed off `tcp`,
     over which no session can present a CN. That over-reports, failing
     closed, and the changelog reads a name's session "as presenting that
     name alone".
2. **Under `deny`, a subject naming both kinds in a near principal's
   policy, off the face.** No 0.5 check reads it:
   - `near_subject_on_face` reads subjects on the face;
   - the face-subject checks read subjects naming no user name and no CN;
   - `face_subject_missing` and `narrowed_too_far` run under `allow`;
   - the denies' reading takes subjects on the face.

   None needs to for §2.4's rule. No session over the face matches it, so
   nothing crosses the face through it. Under `deny`, a session presenting
   one of its names alone matches it not and gets nothing (failing
   closed). One presenting both gets that policy's grants on the near
   side, which is core §11's question: the changelog leaves both "A
   subject bound to no principal and off the face" and "The generator
   without a face policy" there. §2.4's refusal of a principal bound both ways is
   the generator's, with no code. Decided; zk2py files nothing.

### F-122 · ambiguity · link.v1 0.5 scenarios.md §11 expected 7, with core §11.2: what a face subject holds under `allow`

> scenarios.md §11, step 7: "As step 5, under `allow`: the face's subject
> listed in `radio-a`'s policy too." Expected 7: "G and X each receive
> what §1's G receives, and neither's puts reach L: under `allow` every
> rule is a deny, and a subject two policies list holds both's."

> §2.15: "A subject two policies list holds both's rules the same way:
> under `deny`, the allows of each, less the denies of either."

> core §11.2: "Under `allow`, Zenoh does not evaluate allow rules. A grant
> compiles into denies of its complement, regenerated on every contract
> revision."

> core scenarios security.md §2: "Every unauthorized action is blocked,
> except a put on a wildcard key, which R6 discards at the consumer."

The text's reason sets a ceiling, not a level. A subject two policies list
holds the denies of both, so X, which matches only the face's subject,
gets at most what `ground`'s policy leaves it. It gets exactly §1's share
only if `radio-a`'s policy denies nothing that X does.

zk2py's generator compiles each principal's complement for every message
and flow, denying it everything it holds no grant for, which is how
security.md §2 reads ("Every unauthorized action is blocked"). `radio-a`
consumes nothing, so its policy denies:
- egress `put` of its own `air/dropped_total` (`radio-a-deny-2-egress`);
- ingress `query` of `state/peers/x` and of `@op/reset`;
- egress `reply`.

Its policy denies no ingress `declare_subscriber` on the console's
wildcard selectors, since its denies name concrete keys. Measured on N
(§11 step 7): X subscribes, and receives nothing at all, no reply and no
token. G also matches `ground`'s own subject, a separate combination, so
any allow wins, and G gets §1's share. Neither's puts reach L, as
expected. Nothing leaks: X gets less.

Core §11.2 does not say whether a principal's complement covers the
messages and flows it holds no grant for, and the expected observation
turns on it. The reference's X received §1's share, so its `radio-a`
policy left X's subscriptions, pushes and replies alone.
**Resolved:** a guess. zk2py keeps its generator, which security.md §2
supports, and reads expected 7's "each receive what §1's G receives", for
X, as at most what §1's G receives. The counted check says so, and a
`NOTE` records X's numbers. Either the expected line could say "at most",
or core §11.2 could say what a complement covers.

**Status at core 0.31 and link.v1 0.6: resolved by both.** Core §11.2: "A generator MUST deny, in each principal's policy, the keys of the services it does not run that its grants do not name." "It MAY leave the keys of the services the principal runs out of that principal's complement". "A generator that denies them too, per message and flow, is stricter, and conforms. The reference leaves them out." `link.v1` 0.6's expected 7 names both outcomes. For the reference: "X receives exactly what §1's G receives". For zk2py's complement: "where it denies a principal its own keys on the messages and flows its Own grant does not name, `put` toward it among them, X receives nothing at all". zk2py's generator takes the stricter branch. Its §11 step 7 now asserts that branch exactly: X with no sample, token, descriptor or reply, and G with §1's share. A probe on a key no contract declares shows that X's session is live, so a dead session would fail the check.

**Where zk2py's reading of link.v1 0.5 needed a guess, beyond the
findings.**
- **The generator's refusal** (§2.4, no code): `compile_faces` refuses a
  subject naming user names and CNs whenever it compiles a face. zk2py's
  enrollment reader takes a principal's `user` alone, so it never writes
  one.
- **mTLS for §11's step 4**:
  - openssl, P-256 keys;
  - the server's certificate with `subjectAltName=IP:127.0.0.1`;
  - each client's with `extendedKeyUsage=clientAuth`;
  - every TLS session also authenticates by user name and password, as
    "beside its user name" says.
- **A rule without `flows`** is read as both flows, measured, though no
  fixture carries one.

## At core 0.31 and link.v1 0.6 (#609)

Core 0.31 answers F-122 in §11.2, "A principal's own keys":
- a generator MUST deny each principal the keys of the services it does
  not run that its grants do not name;
- it MAY leave the principal's own services' keys out of its complement;
- one that denies them too conforms.

`security.md` §2 names the exception. `link.v1` 0.6 changes only §11's
expected 7, which now gives X's share exactly under each complement. No
rule, code or fixture of either text changed.

**`just py-conformance`: 1414 of 1414,** as in round 34, link 156.

**`just py-link`: 79 of 79,** all eleven sections. §11 has one more check
than round 34.

**§11, step 7, pinned to zk2py's branch.** zk2py's generator (`zk2py.acl`)
compiles a principal's complement per message and flow, over every key of
the deployment it holds no grant for, its own services' keys included.
That is core §11.2's stricter branch. `radio-a`'s policy denies `radio-a`:
- egress `put` of its own stream;
- ingress `query` of its own state and operation;
- egress `reply`.

The face's subject, which that policy also lists, holds those denies.
Step 7 asserts the branch's outcome exactly:
- G receives §1's share through `ground`'s own subject: 5 of
  `air/dropped_total` and of `frames`, none of `air/frames_total` or
  `raw`, all 10 `oper`, 5 `fault`, no `change`, no token or descriptor,
  and its GET and call answered;
- X receives nothing at all: every count 0, no token, no liveliness
  token, no descriptor, and neither its GET nor its call answered;
- neither's 5 puts reach L.

A second check makes a dead X fail: G and X each put once on
`zk2py-probe/<who>`, a key no contract declares, which stays open under
`allow` (core §11.2) and no face deny covers. L receives each, 1 and 1.

**Robustness** (review-bot on PR #790):
- **`check_ids` now covers rules and policies.** Measured on zenoh
  1.10.1:
  - a rule with no id is refused at insert ("missing field `id`");
  - a repeated rule id is refused at open ("Rule id must be unique");
  - a repeated policy id is refused at open ("Policy id must be unique:
    id 'x' is repeated");
  - a policy with no id loads, and so do two.

  zk2py raises `BlockError` for the first three, naming the rule or the
  policy. A policy needs no id, and only the ids present are compared.
- **`subject_denies` and `compare_block`** look rules up with `.get`, so
  a policy naming an unknown rule cannot raise a `KeyError` even before
  `check_ids` runs.
- **`mint_certificates`** raises with openssl's stderr when openssl fails,
  for example: `openssl req -x509 failed (1): req: Can't open "ca.key" for
  writing, Permission denied`.
- **Failed closes are logged once.** `close_each` closes everything,
  continuing past a failure, and prints one line on stderr when any fail
  ("2 of 3 of … failed to close: …, and 1 more"). `_close_all`,
  `World.close` and every run's cleanup use it. No silent `except: pass`
  is left in `link_scenarios`. The full run logged none.

**The question: does core 0.31's new MUST decide, for every message and
flow, what a generator must deny?** Not by itself; it does with
`security.md` §2. That is F-123.

### F-123 · ambiguity · core 0.31 §11.2 "A principal's own keys": keys, or keys per message and flow

> core §11.2: "A generator MUST deny, in each principal's policy, the keys
> of the services it does not run that its grants do not name."

> core §11.2: "It MAY leave the keys of the services the principal runs
> out of that principal's complement … Under `allow`, the principal then
> also reads and receives its own keys on the messages and flows that its
> Own grant's rows do not name." … "A generator that denies them too, per
> message and flow, is stricter, and conforms."

> core scenarios security.md §2: "Every unauthorized action is blocked,
> with two exceptions"

A grant names keys for some messages on some flows (§11.2's table): the
console's Consume grant names `zk2/lab/*/radio.v1/stream/air/dropped_total`
for ingress `declare_subscriber` and `query`, and egress `put`, `delete`
and `reply`. The MUST's object is "the keys … that its grants do not
name".
- **Read per key**, a key any grant names for any message is outside the
  MUST. The sentence alone then requires no deny of `ground`'s ingress
  `put` on `lab/radio-a`'s `air/dropped_total`, which its grants name as a
  key, though not for `put`.
- **Read per message and flow**, it does require one. The MAY and the
  stricter branch beside it are written per message and flow, and
  `security.md` §2 reads "Every unauthorized action is blocked, with two
  exceptions", neither of which is this write.

`link.v1`'s scenarios assume the second reading: §7's expected 3 ("`ground`'s
deny of other services' writes holds for both") and §9's control. The
bullet's own words decide neither.
**Resolved:** a guess, read with `security.md` §2. zk2py's generator
denies per message and flow, as it did. Wording that would decide it:
"deny, on each message and flow, the keys of the services it does not run
that its grants do not name for that message and flow".

**Status at core 0.32: resolved by 0.32, in zk2py's wording.** Core §11.2: "A generator MUST deny, in each principal's policy, on each message and flow, the keys of the services it does not run that its grants do not name for that message and flow. A Consume grant on another service's key names it for reading and receiving only, so the principal's writes to it are denied (the table above)." zk2py's generator already compiled every message and flow apart, and nothing in it changed.

**Where zk2py's reading of core 0.31 and link.v1 0.6 needed a guess,
beyond the finding.**
- **The liveness probe's key**, `zk2py-probe/<who>`, is outside the
  deployment's keys and every face deny. Core §11.2's "Keys no contract
  declares stay open under `allow`" makes it reachable for any session.
- **Policy ids are optional**, measured. A repeated one is refused only
  between policies that carry an id: two policies with none load.

## At core 0.32 (#609)

Core 0.32 rewords one MUST of §11.2, "A principal's own keys", in zk2py's
wording: "on each message and flow, … for that message and flow". That
answers F-123. No rule, code or fixture changed, and `link.v1` stays at
0.6.

**`just py-conformance`: 1414 of 1414. `just py-link`: 79 of 79,** all
eleven sections. §11 also prints the cold read's measurements as `NOTE`
lines (below), which are not counted.

**Robustness** (review-bot on PR #792):
- **The liveness probe** (§11 step 7) no longer depends on fixed sleeps:
  - L's subscriber on the probe key is undeclared in a `finally`;
  - each session puts again every 0.1 s until L has one from every session,
    or until a 5 s deadline. `PROBE_DEADLINE_S` states the assumption:
    a declaration and a put each cross N on loopback well within it;
  - the probe key carries a suffix unique to the run,
    `zk2py-probe-<12 hex>/<who>`.
- **`check_ids` checks a policy id's type** before comparing it.
  Measured on zenoh 1.10.1:

  | Policy id | At load |
  |---|---|
  | a list | refused at insert ("invalid type: sequence, expected a string") |
  | a number | refused at insert |
  | `null` | loads, as an absent id does |
  | `""` | loads |

  zk2py raises `BlockError` for an id that is neither a string nor null.

**The last cold read of `link.v1` 0.6, §2.4, §2.15 and §5.** What a second
implementation would still have to guess comes to two findings, F-124 and
F-125, each measured.

Two more readings are decided by measurement. The text does not state
them, but zenoh's own defaults settle them:
- a rule with no `flows` applies to both flows (round 34);
- a block with no `default_permission` behaves as `deny`. Measured with
  the generator's `deny` block without it: G got §1's share and X nothing.

Neither is in `lints.json`'s shape, and a tool reading a live block needs
both. That is the same gap as F-124, which zk2py files once, for the
default that hides a whole block.

Everything else read as decided.

### F-124 · gap · link.v1 0.6 §2.15, §5: a block whose access control is not enabled

> §2.15: "A tool checks a block read from a router's configuration against
> the face it guards. It MUST report what the fixture reports"

> §5: "a router's block is a file someone read, since the running block is
> not on the bus (core §11.3)." … "**Unobservable:** whether the router
> runs the block read (core §11.3)."

> `lints.json`'s description: "block is a router's block for one face:
> {face, namespace, deny: [{flows, messages, key_exprs}], downsampling:
> [{key_expr, freq}]}"

zenoh's `access_control` carries `enabled`. A block whose `enabled` is
false enforces none of its rules. So does a block that omits it, since
false is zenoh's default (measured).

Measured with the generator's `deny` block for §7's deployment:
- with `enabled: false`, and again with `enabled` absent, G and X each get
  `local`'s share: 100 of `air/frames_total` and of `raw`, the 5 `change`
  occurrences, both tokens and the descriptor;
- each one's 5 puts on `lab/radio-a`'s key reach L;
- zk2py's tool reports nothing, since every check reads the rules and the
  subjects, which are all in place.

§5's "Unobservable: whether the router runs the block read" is about a
router that may not run the file. Here the file itself says it is off,
which a tool can read. The text, `lints.json`'s shape and its codes say
nothing of `enabled`. A second implementation must guess whether a
disabled block is checked as written (zk2py's tool does, and passes it),
reported, or "not asked".
**Resolved:** open. zk2py's tool checks the rules as written, as the
fixture's shape implies. A reading that would decide it: a finding for a
block whose `enabled` is not true (none of its rules is enforced, and
under either posture everything crosses). Or §2.15 could say a tool reads
`enabled`, `default_permission` and a rule's `flows` as zenoh 1.10.1
defaults them: false, `deny`, and both flows.

**Status at link.v1 0.7: resolved by 0.7, both of the readings zk2py offered.** §2.15: "`block_disabled` (error, `at` `enabled`; 0.7, F-124): the block's access control is not enabled: its `enabled` is false, or absent from the router's configuration, which zenoh 1.10.1 reads as false." "It is reported beside the block's other findings, which read the block as written". And "What a block omits": "A tool reading a router's configuration reads what the configuration leaves out as zenoh 1.10.1 does, each measured `[Sc: §12]`: `enabled` absent as false, a block's `default_permission` absent as `deny`, and a rule's `flows` absent as both flows." `lints.json`'s five new `block_disabled` cases pass, and zk2py's scenarios §12 run the four blocks live.

### F-125 · gap · link.v1 0.6 §2.4, §2.1: two faces with one far principal

> §2.4: "A generator MUST therefore put every deny of a face's plan into
> the policy of each subject that can match the far principal's session
> on the face: the far principal's, at least."

> §2.4: "The far principal's subject is narrowed off the faces whose far
> principal it is not, and no further."

> §2.3: "A generator derives a plan for each face of a policy"

Nothing keeps two faces of one policy from naming the same `far`.
- §2.1's linter has no finding for it.
- Appendix B's zenoh-modem has an RF face whose far principal is `ground`,
  and an SBD terminal whose policy it does not show. On one router with
  one ground principal, the two would be this case.
- `plan.json` and `lints.json` give two faces distinct far principals.

The plans are per face. But a zenoh rule cannot be limited to a link
protocol, and the far principal's subject is narrowed off no face of its
own. So both plans' denies sit in one policy, which holds wherever its
subject matches, on either face and off them. Deny is evaluated first
within a policy, so each face carries the union of the two plans' denies.
That fails closed: no leak. But the face that exposes loses what its plan
lets cross.

Measured on N, under `deny`: the scenarios' policy with a second face,
`sat`, to `ground` over `tls`, exposing none. zk2py's generator compiles
both faces into `ground`'s policy, `face-radio-*` beside `face-sat-*`. G
over the radio face then receives no sample of `air/dropped_total` or
`frames`, no `oper` put and no `fault` occurrence, where §1's G receives
5, 5, 10 and 5. Its GET and call are still answered (by grant, §2.9). The
tool reports nothing: the block holds every deny of both plans, and §2.15
reads no plan.

A second implementation must guess among:
- refusing the policy;
- accepting the union;
- splitting the far principal's subject per face: one subject per face's
  link protocols, each in a policy with that face's denies, and under
  `allow` one more off the faces.

The text describes none of these. Splitting would pass §2.15's checks,
but §2.4's "and no further" can be read against it.
**Resolved:** open. zk2py's generator writes the union, as §2.4's MUST
reads, and the scenarios never give two faces one far. A reading that
would decide it:
- a lint for two faces with one `far`, refused or warned;
- or §2.4 saying the far principal's policy holds every face's denies, so
  each such face carries all of them;
- or §2.4 describing the split.

**Status at link.v1 0.7: resolved by 0.7, the union, warned.** §2.4: "A far principal of two or more faces (0.7, F-125) carries every one of their plans' denies on each of them: the stricter, failing closed." "A generator writes the union, and reports the linter's warning (`far_of_two_faces`, §2.1) with its plans: it refuses nothing, since nothing leaks." §2.1: "`far_of_two_faces` … Two or more faces of the policy name the same `far` (0.7, F-125): each of them carries every one of their plans' denies (§2.4). One finding per such name, with no face; `at` is the name." It "reads every face whose `far` is a non-empty string, whatever else the policy or the face holds". zk2py's generator already wrote the union. Its linter now warns, cases 30, 31, 60 and 118 to 125 pass, and scenarios §13 runs both faces live.

**Where zk2py's reading of core 0.32 and link.v1 0.6 needed a guess,
beyond the findings.**
- **The probe's deadline**, 5 s, and its 0.1 s repeat: timings for
  loopback. A probe later than the deadline counts as not arrived.

## At link.v1 0.7 (#609)

`link.v1` 0.7 answers F-124 and F-125.
- **`block_disabled`**, a §2.15 error at `enabled`. A router's
  configuration without `enabled` reads as false, and `lints.json`'s
  absent `enabled` means it was not read.
- **`far_of_two_faces`**, a linter warning with no face. A far principal of
  two faces carries every one of their plans' denies on each.
- **zenoh's defaults are stated:** access control off or absent enforces
  nothing, no `default_permission` is `deny`, and a rule without `flows`
  applies to both.
- **Scenarios §12 and §13.**

The core stays at 0.32.

**`just py-conformance`: 1427 of 1427,** with no unknown file. The `link`
family is 169: `exposure.json` 31, `plan.json` 12, `lints.json` 126 (13
new, and cases 30, 31 and 60 now expecting `far_of_two_faces`). Every case
passed the first time it ran.

**`just py-link`: 89 of 89,** all thirteen sections. §12 and §13 are
counted checks, in place of round 36's notes.
- **§12, what a block omits** (the generator's `deny` block, hand
  edited):
  - **step 1, `enabled: false`:** nothing is enforced. G and X each
    receive 100 of `air/frames_total` and of `raw`, all 10 `oper`, 5
    `fault` and 5 `change`, both tokens and the descriptor, and their GET
    and call are answered. The downsampling still applies: 5 of
    `air/dropped_total` and of `frames` each. All 5 of each one's puts
    reach L;
  - **step 2, the `enabled` line taken out:** the same;
  - **step 3, `default_permission` taken out:** G gets §1's share. X gets
    nothing at all, no token and no reply, and neither's puts reach L;
  - **step 4, `flows` taken out** of the console's Consume rule for the
    samples it receives (`ground`'s and `local`'s, one per principal in
    zk2py's block) and of every face deny. G gets §1's share and nothing
    the face denies. Of its puts, 5 of 5 on `air/dropped_total` reach L
    and 0 of 5 on `air/frames_total`. Under the generator's block, 0 of
    each;
  - **step 5, the tool:** steps 1 and 2, `block_disabled` at `enabled`
    alone; steps 3 and 4, no finding. zk2py's comparison with the plan,
    which reads denies only, finds nothing for any step.
- **§13, two faces, one far principal**, under `deny`, `sat` over `tls`
  exposing none, N not listening on TLS:
  - **step 1:** the linter warns `far_of_two_faces` at `ground` and refuses
    nothing. `ground`'s policy holds `face-radio-*` and `face-sat-*`. Over
    the radio face G receives 0 of `air/dropped_total`, of `frames`, of
    `oper` and of `fault`, sees no token and no descriptor, and has its GET
    and call answered;
  - **step 2 (control, `sat` to `ground-router`):** no warning.
    `ground-router`'s policy holds `face-sat-*`, and `ground`'s
    `face-radio-*` alone. G gets §1's share;
  - **step 3:** no finding for either block.

**Robustness** (review-bot on PR #794):
- **Worlds close on error.** A `World` whose start fails after N opened
  closes what it opened, N among it, before the error goes on, and its
  `close` takes a partial world. `_run7`'s worlds close in a `finally`, as
  every run's do.
- **Note-only worlds are isolated** (`noted`): §7's measurement of step
  2's block under `allow` reports a failure as a `NOTE`, not a crash.
  Round 33's F-121 notes in §9 are gone, since §11's steps 1, 2 and 4 count
  the same measurements. Round 36's cold-read notes in §11 are now §12 and
  §13.
- **The liveness probe** records, per session, `arrived` beside
  `attempts` (the puts it took).
- **A bool policy id** is refused at insert ("invalid type: boolean
  `true`, expected a string"), and zk2py raises `BlockError`, as for a
  list or a number. Measured beside it, an `enabled` that is not a bool is
  refused at insert too ("expected a boolean": a string, `1`, `null`).
  zk2py's tool raises `BlockError` for one, since no router runs it.

**Two places where zk2py and the reference may differ.** No fixture tells
them apart. Both are decided by §2.1's "Each key is checked alone. A key
whose value is wrong is reported once, and checks that need it are
skipped", and zk2py follows it:
1. **`batch_over_lease`** needs `batch_bytes`, `link_bps` and `lease_ms`
   (10,000 when absent), and nothing else. It is computed whenever those
   three are right, whatever else the face holds. zk2py: a batch over the
   lease beside a wrong `class` gives `batch_over_lease` and `bad_value`
   at `class`. A face policy refused by its errors still carries its
   warnings, as cases 30, 122 and 123 show.
2. **`shared_protocol`** needs `select.link_protocols`. A list naming a
   protocol zenoh does not name is a wrong value: "a non-empty array of
   zenoh 1.10.1's link protocol names". So it is reported once
   (`unknown_protocol`), and the check that needs it is skipped, for the
   other face too, since the pair needs both. zk2py: faces `a`
   `["tls", "lora"]` and `b` `["tls"]` give `unknown_protocol` at `a`
   alone. Both known give `shared_protocol` at each.

   `far_of_two_faces` shows that the text names an exception where it
   wants one: it "reads every face whose `far` is a non-empty string,
   whatever else the policy or the face holds". `shared_protocol` has no
   such exception.

If the reference reads either otherwise, the text says it should not.
zk2py files nothing.

**The last cold read of `link.v1` 0.7, §2.1, §2.4, §2.15 and §5:** nothing
left that a second implementation would have to guess. One value lies
outside what the text reads, and no router runs it: an `enabled` that is
not a boolean, which zenoh refuses at load (measured above).

**Where zk2py's reading of link.v1 0.7 needed a guess.**
- **`far_of_two_faces` counts a face whose name is not a plain chunk**,
  as "every face whose `far` is a non-empty string, whatever else … the
  face holds" reads. No case has one.
- **A non-boolean `enabled`** raises `BlockError` (refused at load,
  measured), where `lints.json` defines `false`, `true` and absent only.

## At alarms.v1 0.1 (#609)

`alarms.v1` 0.1 is the sixth profile (`spec/profiles/alarms/`). zk2py read
it cold with core 0.33 and 0.34. Its standard contract moved from
`examples/zk2/` unchanged, and revision 1.0 is in both history roots.
`zk2py.alarms` is the reader's and the tool's half, from the text alone:
- the key recipe through the sanitizing step, and §2.12's reference;
- the document's rules;
- reading transitions;
- §2.14's, §2.15's and §5's conclusions.

**Core 0.33** (§11.1): a generator MUST refuse an input that binds one
principal by both a user name and a CN. zk2py needed no change:
- its input (`acl.Principal`) binds a principal by its user name alone and
  has no CN, so no input can bind one principal by both. `acl.py` now says
  so;
- `link.compile_faces`, which reads a block the generator wrote, still
  refuses a subject with both on a router with a face (`link.v1` §2.4).

**Core 0.34** publishes `alarms.v1`'s vocabulary, empty. zk2py moved
`alarms` from its interim table to its published one, with no key, so
`contracts/w105-alarms-vocabulary` draws W105.

**`just py-conformance`: 1715 of 1715.** At the start of the round, 1432
passed and 2 failed: `layout` (the profile's `conformance/` unread) and
`contracts` (`w105-alarms-vocabulary`).

| Family | Before | After |
|---|---|---|
| layout | 18 of 19 | 19 of 19 |
| contracts | 100 of 101 | 101 of 101 |
| alarms | (none) | 281 of 281 |
| the 17 others | 1314 of 1314 | 1314 of 1314 |

The `alarms` family covers four files, every case passing the first time
it ran:
- `keys.json`, 120 checks: its 75 cases, each `derive` also against the
  strict recipe, and each parsed key built back;
- `documents.json`, 49;
- `transitions.json`, 24;
- `conclusions.json`, 88.

`examples` stays byte-identical: `alarms.v1.toml` builds its published
bundle, and the two history roots hold one file.

**Passing is not deriving.** Two kinds of expectation come from the
fixture, not the text:
- the reason §5's third question gives a delete, `no_document` (F-132);
- the reasons §2.15 gives an owner that fails §2.14's steps 1 and 2
  (F-130).

And `conclusions.json` passes under two readings of §2.14 step 4, which
answer a live reading differently (F-128).

### F-126 · gap · alarms.v1 0.1 §2.4: how a payload becomes a document

> §2.4: "A reader MUST read a payload outside the schema, or with a time out
> of that range, as undecodable"

> §2.2: "The labels are a JSON object, whose names are distinct"

The schema judges a JSON value. The text does not say how a payload's bytes
become one, so three cases are left to the JSON parser:
- **A duplicate member:** `"severity"` twice, or a label name twice.
  Parsers keep the first, keep the last, or refuse. So two readers can read
  two severities from one put, or derive two keys. §2.2's "whose names are
  distinct" holds only for a parser that refuses duplicates.
- **`NaN` and `Infinity`,** which some parsers accept.
- **A lone surrogate escape** (`"\ud800"`). It is JSON text but has no
  UTF-8 form, so §2.2's "the input's UTF-8 bytes" do not exist.

The core reads its own documents as strict JSON: a descriptor (D000, "a
duplicate member included") and a schema (E029). It has no such rule for a
payload.

**zk2py:** strict JSON (`alarms.decode`, through `jcs.loads`). A duplicate
member, a non-finite number or a string with no UTF-8 form does not read,
so the payload is undecodable. No fixture has one, since `documents.json`
gives its payloads already decoded.

**Wording that would remove the guess:** §2.4, "A reader reads a payload
that is not strict JSON (RFC 8259 with no duplicate member, every string
valid Unicode) as undecodable, as core §3.3's D000 reads a descriptor."

**Status at alarms.v1 0.2: resolved by 0.2, as zk2py read it.** §2.4,
"Strict JSON first": "a payload's bytes as core §3.3's D000 reads a
descriptor: UTF-8, JSON (RFC 8259) with nothing after its value, no
duplicate member at any depth, every number finite, and every string with
a UTF-8 form … A payload that is not strict JSON is undecodable." §2.2's
"whose names are distinct" now cites it. `documents.json`'s nine `text`
cases (token `json`) pass through `alarms.strict_json`, which is
`alarms.decode`'s reading from round 38. One phrase is left to read:
"nothing after its value" (F-134).

### F-127 · ambiguity · alarms.v1 0.1 §2.6: "equal", with `labels` absent or `{}`

> §2.6: "**Equal** is equal in every member this text names (§2.4)."

> §2.4: "**`labels`**, optional: an object of strings."

A document without `labels` and the same document with `"labels": {}`
derive one key and say the same thing. A reader holding one reads the other
as **confirmed** if they are equal, and as **refreshed** if not.
- A typed reader that defaults a missing map to empty reads them as equal.
- A reader that compares members as JSON reads them as unequal.

`observed_at_ms` and `category` raise no such question: an empty value of
theirs is not their absence.

**zk2py:** unequal, since an absent member is absent. `transitions.json` has
no such pair. Neither answer is a transition, so no acknowledgement turns on
it, but a tool that counts refreshes would.

**Wording:** §2.6, "`labels` absent is equal to `{}`", or "a member absent
is equal only to itself absent".

**Status at alarms.v1 0.2: resolved by 0.2, against zk2py's reading.**
§2.6: "`labels` absent is equal to `labels: {}` … Any other member absent
is equal only to itself absent, so `category: ""` is not an absent
`category`." zk2py now reads an absent `labels` as `{}` (`Alarm.labels`).
`transitions.json`'s three new cases failed under round 38's reading (two
of them) and pass now.

### F-128 · ambiguity · alarms.v1 0.1 §2.14 step 4, with §2.15: what a GET after a loss heals

> §2.14 step 4: "**A loss.** A subscription lost deliveries, and no GET
> answered since: **unobservable**, `incomplete`. A lost delivery may have
> been this key's latest, so a reader that lost one re-seeds by S4's GET
> (core §4.2)."

> §2.15: "a reader's S4 GET … ran to its final reply with no error reply …,
> the reader's subscription, if it has one, lost no delivery and its owner
> was present throughout"

"No GET answered since" reads two ways:
1. no GET has answered **this key** since, with a value or a `reply_del`;
2. no GET has **run** since, to its final reply.

They part when three things hold: the subscription lost deliveries, its
last put of the key is fresh, and a complete GET since drew nothing for the
key.
- Reading 1 stops at step 4: **unobservable**, `incomplete`.
- Reading 2 goes on to step 6, where the subscription's put is fresh:
  **yes**, `firing`. Yet by core S2, that complete GET says the owner holds
  no live key there.

`conclusions.json` does not tell them apart. Its one such case, "a loss,
and a GET that drew nothing for the key", has its subscription's put 1000 s
old, so `freshness.v1` reads it `incomplete` either way. zk2py ran all 88
cases under reading 2 too, and all 88 pass. With that put 10 s old,
reading 1 answers `incomplete` and reading 2 `firing`.

§2.15 has the same gap from the other side. Its subscription must have
"lost no delivery", with no "since":
- a loss that a later complete GET re-seeded still leaves the owner
  `window_lost`;
- so "established" never returns on that subscription.

`owner_set`'s inputs carry one `lossless` flag and no order, so no case
decides this.

**zk2py:** reading 1, the narrower one: a GET heals a loss for a key it
answered. For §2.15, it takes `lossless` as given.

**Wording:**
- §2.14 step 4: "and no GET has answered this key since, with a value or a
  `reply_del`";
- §2.15: whether a complete GET after a loss re-establishes the owner: for
  instance, "lost no delivery since that GET, and for `window`, over the
  window".

**Status at alarms.v1 0.2: resolved by 0.2, reading 1, zk2py's.** §2.14
step 4: "no GET has answered this key since, with a value or a
`reply_del` … A GET that drew nothing for the key, complete or not, does
not heal the loss for it". §2.15, "The window": "from its declaration, or,
after a loss, from the first complete GET that followed it … which heals a
loss before it and not one after it". `conclusions.json` pins the case
zk2py named, a put 10 s old: `incomplete`. zk2py's live reader never knows
of a loss (a zenoh-python callback subscriber has no queue), so its window
always runs from the declaration, and a stall or a closed transport makes
it lossy (`WindowWatch`).

### F-129 · gap · alarms.v1 0.1 §2.14 step 5: an earlier instance, told by a stamp's id

> §2.14 step 5: "a put stamped by an earlier instance of the owner than the
> present one, its stamp's id not the present descriptor's `meta.zid` (core
> §4.3, §3.3): **not asked**, `previous_instance`. It is that instance's
> last word"

The step tells an instance by a zid, but an instance is an instance id
(core §1.5), and the two need not move together:
- **A re-mint in one session.** Core §1.5 only says an owner SHOULD
  re-mint in a new session. One that keeps its session keeps its zid. Its
  earlier instance's put then passes for the present one's, and nobody can
  check §2.9's re-put.
- **No `meta.zid`.** It is a SHOULD (core §3.3: "Without it, a stamp's
  clock is unattributable, never foreign"). Then the step's test has
  nothing to compare with. The text says neither "every put is the present
  instance's" nor "none is".
- **A stamp that is no instance's of the owner:** a router's re-stamp of a
  put from a session without an HLC, or any writer breaking S1. Core §4.2
  says "A foreign stamp is a finding". Step 5 instead reads it as
  `previous_instance`: not asked, shown as last-known, and no finding.

**zk2py:** `Reading.previous_instance` is the caller's input, as the
fixture's is. zk2py's live reader will set it when the stamp's id, compared
by value, is not `meta.zid`. It will leave it unset when the descriptor has
no `meta.zid`, by core §3.3's "never foreign". It cannot see a re-mint in
one session.

**Wording:** "a put whose stamp's id is not the present descriptor's
`meta.zid`, by value: **not asked**, `previous_instance`, whether an
earlier instance stamped it or another session did (core §4.2's finding).
With no `meta.zid`, step 6. A re-mint that keeps its session is not told
apart."

**Status at alarms.v1 0.2: resolved by 0.2, all three cases stated.**
§2.14 step 5: the id compared "by value"; another session's stamp reads
`previous_instance` too, "also core §4.2's foreign stamp, a finding a tool
reports apart"; with no `meta.zid`, "the reader goes on to step 6"; a
re-mint that keeps its session "is not told apart" (§2.9 too). zk2py's
reader (`read_owner`) does each: `previous_instance` by value, step 6
without `meta.zid`, and the GET replies stamped by another session listed
apart (`OwnerReading.foreign`), which scenario §5 step 4 asserts empty.

### F-130 · gap · alarms.v1 0.1 §2.15 "Established": the reasons, a face, and their order

> §2.15: "**Established.** An owner's alarms are known whole,
> **established**, when it is present and listed (§2.14, steps 1 and 2), …"

> "**Otherwise unobservable:** no GET (`no_seed`); one that ended at its
> timeout or with an error reply (`seed_incomplete`); a window that lost a
> delivery (`window_lost`), or over which the owner was not present
> throughout (`owner_gap`); an empty GET and no window (`no_member` …), or a
> window no longer than 900 s (`window_short`); across a face, an empty GET
> (`across_face` …). Absent and not listed are **not asked**, as in §2.14."

The fixture expects three things the text does not state.
1. **The reasons of steps 1 and 2.** The list has no reason for presence
   possibly incomplete, a descriptor not read, or a face closed.
   `conclusions.json` expects `presence_incomplete`, `no_descriptor` and
   `face_closed`, which are §2.14's reasons. zk2py carried them over.
2. **Across a face.** No owner across a face is "present and listed":
   presence does not cross, and the deployment's word stands for the
   descriptor. Read as written, no owner across a face is ever established.
   Yet the fixture establishes one ("across a face, a GET by grant that the
   owner answered", `answered`). That follows only if "present and listed
   (§2.14, steps 1 and 2)" means "§2.14's steps 1 and 2 do not answer".
   - The fixture's across-face case with an empty GET carries a window with
     `present_throughout: true`. The text says no window across a face can
     have that ("no window is of a present owner").
   - A reader that sets it false across a face, as the text suggests, makes
     every across-face owner with a subscription `owner_gap`, `answered`
     included.
3. **The order.** The reasons overlap, and the text never says which wins:
   - a timed-out GET and a lossy window: `seed_incomplete` or
     `window_lost`;
   - across a face, an empty GET and no window: `across_face` or
     `no_member`.

   No case has two at once.

**zk2py:** `alarms.owner_set` checks, in this order:
1. §2.14's steps 1 and 2, with their reasons;
2. `no_seed`, `seed_incomplete`, `window_lost` and `owner_gap`;
3. `answered`;
4. `across_face`, `no_member` and `window_short`;
5. `window`.

Across a face, it takes a window's `present_throughout` as given.

**Wording:**
- "established when §2.14's steps 1 and 2 do not answer";
- "otherwise, by the first of these that holds: §2.14's steps 1 and 2,
  with their reasons; …";
- for a face, either "a window across a face is never present throughout",
  or "across a face, `answered` needs no window".

**Status at alarms.v1 0.2: resolved by 0.2, zk2py's order, with one change.**
§2.15, "The order": §2.14's steps 1 and 2 with their reasons; the seed;
the window (`window_lost`, then `owner_gap`); `answered`; across a face,
an empty GET; `no_member` and `window_short`; then `window`. "Present and
listed" now reads "§2.14's steps 1 and 2 do not answer". The change:
"Across a face, presence does not cross, so whether the owner was present
is not asked there", which round 38's `owner_set` asked (`owner_gap`).
zk2py follows; two of the six new cases failed before it did.

### F-131 · gap · alarms.v1 0.1 §2.15, with scenarios §8: which owners are in scope

> §2.15: "**Anything firing at or above a severity *S*,** over the owners in
> a scope … Each owner is counted by its verdict: established, unobservable,
> not asked."

> §2.13: "A reader MUST find `alarms.v1`'s owners through instance tokens
> and descriptors"

> scenarios §8 step 3: "T's answer to "is anything firing at or above
> `warning`?" over `zk2/lab/*` counts the owner as not asked"

An owner found through instance tokens is present. Then an absent owner is
never in scope, and "not asked" counts only the owners whose descriptors do
not list `alarms.v1`. Scenario §8 expects the absent `lab/svc` to be
counted, so T must know it some other way:
- its own step 1;
- an archive's keys;
- a list of the services it expects.

The text names none of them.

**zk2py:** `any_firing` counts the owners its caller gives. zk2py's §8 will
count an owner that T read earlier in the same run, and nothing else.

**Wording:** "The owners in scope are those found by presence (§2.13),
and those the tool knew of before: by an earlier reading, or by an
archive's keys (core S6). An owner known only the second way is absent:
not asked."

**Status at alarms.v1 0.2: resolved by 0.2, as zk2py did it.** §2.15, "The
owners in scope": "those its presence read finds … and those it knew of
before, by an earlier reading of its own or by an archive's keys (core
S6). An owner known only the second way … is absent: not asked."
`scenarios.md` §8: "T knows of the owner from its step 1". zk2py's §8 runs
exactly that, on both owners.

### F-132 · gap · alarms.v1 0.1 §5: the third question's reasons for a delete and an undecodable payload

> §5, "Does this key re-derive from its document?": "**Not asked:** a
> delete, which carries no document. **Unobservable:** the payload
> undecodable."

`conclusions.json` expects `{"answer": "not_asked", "reason":
"no_document"}` and `{"answer": "unobservable", "reason": "undecodable"}`.
- The row names `strict`, `sanitized` and `mismatch` as reasons, but not
  these two.
- `no_document` occurs nowhere in the text, nor in the fixture's
  description.

zk2py passes the delete's case only by copying its expectation. Its own
guess would have been `deleted`, as in §2.14. `undecodable` it could guess
from §2.14.

**Wording:** "**Not asked:** a delete, which carries no document
(`no_document`). **Unobservable:** the payload undecodable (`undecodable`)."

**Status at alarms.v1 0.2: resolved by 0.2.** §5's third row names
`no_document` and `undecodable`, and so does the fixture's description.

**Where zk2py's reading of alarms.v1 0.1 needed a guess, beyond the
findings.** No fixture case reaches these:
- **A not-asked freshness at step 6.** This takes observations that
  contradict the latest put: a firing put, with every observation an
  archive or a delete. §2.14 step 6 names fresh, stale and unobservable.
  zk2py answers unobservable, with `freshness.v1`'s reason.
- **The last question when the put does not read and its freshness is
  unobservable** (near the horizon, say): zk2py answers `undecodable`, the
  line its step 6 puts first. The first question gives `freshness.v1`'s
  reason.
- **An archive's document and `latest`.** §0's reading takes the latest
  sample from "a subscription and a GET merged". So an archive's document
  is not the latest, yet the fixture's archive cases write `latest:
  firing`. No answer turns on it, since steps 1 and 3 answer first.
  zk2py's live reader will follow §0.

**The scenarios** (`scenarios.md` §1 to §10), read for a later live round:
- **§1 to §4, §6, §7, §9 and §10** can be implemented from the text.
  zk2py's owner will need three additions: to stop confirming one alarm
  while it keeps its tokens (§6), a stated cardinality in its descriptor
  (§7), and a refused raise told to its writer (core §2.7).
- **§5** leaves two readings of S unsaid, which §2.6 decides:
  - R's and P's deletes read **retired**, since S never held either;
  - B's unchanged re-put reads **confirmed**, before its **resolved** and
    **retired**.

  The persistent record's format is the owner's own.
- **§8** needs F-131's answer. Its archive will be zk2py's stand-in for
  `archive.v1`, as in `health.v1`'s §6.

## At alarms.v1 0.2 (#609)

`alarms.v1` 0.2 answers F-126 to F-132; each entry above carries its
status. It also states three of round 38's other guesses as zk2py had made
them, and settles five places the reference's owner met. The core stays at
0.34.

**zk2py's changes for 0.2.**
- **§2.4, strict JSON first** (`alarms.strict_json`, `check_payload`):
  `documents.json` gives nine cases as their `text`, refused as `json`
  before `schema`. This is round 38's `decode`, now a separate step.
- **§2.6:** `labels` absent equals `{}` (F-127, the one answer against
  zk2py).
- **§2.15:** across a face, a window's presence is not asked (F-130).

**`just py-conformance`: 1745 of 1745** (after review of PR #807: 1734
from the fixtures, and 11 of zk2py's own readings of F-134 in a family
of their own, `readings`).
- At the start of the round, the `alarms` family stopped at
  `documents.json`'s first `text` case, which round 38's runner did not
  read (exit 2).
- With those cases read through round 38's `decode`, 296 of 300 passed. The
  four failures were the two new F-127 cases and two of the six new F-130
  cases.
- The `alarms` family is now 300 checks: `keys.json` 120, `documents.json`
  58, `transitions.json` 27 and `conclusions.json` 95. Every other family
  is unchanged.

**The live half: `just py-alarms`** (`python -m zk2py.alarms_scenarios`).
It runs `scenarios.md` §1 to §10 against zk2py's owner, or, with
`--owner <exe>` (or `ZK2PY_OWNER`), against the Rust owner example as a
black box. The example is `owner-ab`, the build at the text 0.2 commit. It
is driven by its flags and its `alarm` stdin commands (F-133), and it
takes the short-ttl copy as a contract file. Each section runs on an
in-process router R1 of its own, with S, G and T as clients. Three
sections wait out the horizon, so by default they run on a copy of the
contract with `freshness.ttl_s = 4`, and `--full` runs them at 900 s:
- §2 waits three confirmation periods, 455 s at 900;
- §6 waits ttl + 1.5 s, 905 s at 900;
- §10 waits ttl + 1 s, 905 s at 900.

| § | What | zk2py's owner | The Rust owner example |
|---|---|---|---|
| 1 | Raise; the duplicate; one key whatever the label order | 9 of 9 | 9 of 9 |
| 2 | Escalate, refresh, confirm | 9 of 9 at ttl 4, and 9 of 9 at 900 s (re-put after 449.9 s) | 9 of 9 at ttl 4, and 9 of 9 at 900 s (re-put after 427.5 s) |
| 3 | Resolve, delete within 1 s, `reply_del`, nothing 65 s later | 6 of 6 | 6 of 6 |
| 4 | Re-raise, and its confirmations (ttl 4 s) | 3 of 3 | 3 of 3 |
| 5 | A restart mid-incident, from the owner's record, and its adoption | 11 of 11 | 11 of 11 |
| 6 | A hung owner goes stale, and an extra check of 0.2's §2.7 resumption | 6 of 6 at ttl 4 | 6 of 6 at ttl 4 |
| 7 | At the bound of 2 | 5 of 5 | 5 of 5 |
| 8 | An absent owner, last-known, out of scope by presence | 7 of 7 | 7 of 7 |
| 9 | A key that does not re-derive (T against a zk2py liar) | 3 of 3 | 3 of 3 (the same liar) |
| 10 | Anything firing, a tokenless owner | 7 of 7 at ttl 4 | 7 of 7 at ttl 4 |

Each column totals 66 of 66 on the default run, and exits 0.
- §4's third check, added after review of PR #807, goes beyond
  `scenarios.md`. A resolve closes the alarm's writer, since a resolved
  alarm is never confirmed. The re-raise's put reopens it: `Owner.set_state`
  clears the writer's closed mark, whatever the act. The check shows two
  confirmations after the re-raise, unchanged, 1.8 s and 1.9 s apart at
  ttl 4.
- So §4 runs on the short-ttl copy too, unless `--full`.
- The runner exits 2 for any check or section not run, unless `--only` or
  `--quick` asked for that subset. §2 also ran at 900 s,
alone, on each owner: 9 of 9, about 7.6 minutes each.

**Not run:**
- **§6 and §10 at 900 s.** Each takes about 15 minutes, beyond the
  10 minutes a command here may take in the foreground. They ran on the
  4 s copy, as the reference's default run does for §2 and §6.
- **§9 against the Rust owner.** Its subject is T, zk2py's tool. The
  owner example has no way to put a document on another key, and the text
  gives an owner none, so the liar is a zk2py owner putting raw documents.
  The section runs identically in both modes.

**What running both owners found in zk2py's own owner.** The text was not
at fault in any of these.
- **Templated state had no re-puts.** zk2py's owner registered a horizon
  only for parameterless state. `alarms/{alert_key}` is the first
  templated state with a horizon it has run, so its alarms would never
  have been confirmed. freshness.v1 §2.4 covers every member, and
  `Owner` now re-puts a templated state's members from their first put.
- **0.2's §2.7, "An evaluation that finds the condition again confirms it
  again".** zk2py read a duplicate observation as no act (§2.5) and left
  stopped confirmations stopped.
  - Measured on the owner example: after `alarm confirm <k> off`, an
    unchanged `alarm raise` puts at once, then every 1.9 s at ttl 4.
  - zk2py now reopens the writer the same way.
  - §6 has an extra check of it, beyond `scenarios.md`, which both owners
    pass.
- **§2.15's window** is now core §2.7's (0.25), as §2.15 cites it.
  `WindowWatch` establishes lossless from the QoS and the reader's receive
  path, and checks the owner's instance token one lease and the reply
  timeout past the window's end. Before, zk2py read the subscriber's own
  flag and never waited.

**Measured beside the text, with no finding:**
- Both owners put `application/json`, strict JSON with no whitespace
  around the value.
- Both put the duplicate raise of §1 as nothing.
- At 900 s, the owner example re-puts after 427.5 s and zk2py after
  449.9 s. Both are within 450 s, and `freshness.v1` §2.4 fixes no margin.
- The owner example's record is keyed by alert key, not by the full key
  the text's SHOULD names. Within one owner's address the two are one.
- In §5, both owners put A again and delete R's and P's keys before the
  new instance token, the text's SHOULD, and both adopt only after their
  deletes and resolves (0.2's order).

### F-133 · gap · the owner example's `--alarms` (#800, AB): its stdin surface is undocumented

Not the spec: the reference's example, which this round was to drive as a
black box from "the binary's own help (`owner --help`) and `scenarios.md`".

`owner --help` prints one usage line. It names `--alarms`,
`--alarms-record <file>`, `--alarms-archive <system>/<service>` and
`--alarms-rules <rule>[,<rule>...]`, and says nothing of the rest:
- **The stdin commands.** The command word is `alarm`, which zk2py found
  by trying words. `help`, `alarms`, `alarms raise` and bare `raise` all
  answer `refused …: unknown command`. Only a malformed `alarm` command
  prints the command's usage: `alarm raise <json> | resolve <alert_key> |
  confirm <alert_key>|all on|off | list | adoption`.
- **The condition's JSON**, `{rule, severity, summary, labels?,
  category?}`. It shows only in a refusal of a malformed one.
- **The answers**, which a runner must parse:
  - `alarm raised|escalated|refreshed|unchanged <alert_key> <document>`;
  - `alarm resolved …`;
  - `refused <line>: <reason>`;
  - `alarm adoption source=… adopted=… retired=<key>:<why>,… resolved=…
    over_bound=…`.
- **That a short-ttl copy is passed as a contract file** (it is, and its
  fingerprint is zk2py's in-memory copy's). The round's instructions said
  the help mentions this; it does not.
- **The record file's format**, `{"profile": "alarms.v1", "alarms":
  {<alert key>: <document>}}`. §5 needs it to write R and P, and zk2py read
  it from the file the example wrote.

`scenarios.md` documents none of it. The health and config surfaces have
the same shape (a usage per command word, reached only by an error).

**What would fix it:** `--help` listing each command word with its usage,
its answers, and that a contract file named `alarms.v1` replaces the
embedded one. Or a section in `scenarios.md` naming the example's commands
per step, as its conventions name the reference's test file.

**Status at alarms.v1 0.3: resolved.** `owner --help` (the 0.3 build,
`owner-ac`) exits 0 and prints the interface whole:
- what standard output carries, and the refusal forms;
- each command word, and for `alarm` each command;
- the condition's JSON, every answer line, and the adoption line's
  `retired=<key>:<resolved|undecodable|phantom>`;
- the record's format, and to write into it only while no owner runs on
  it;
- that an `alarms.v1` contract file of another revision is implemented on
  it ("every 1.9 s at 4 s");
- `--cardinality alarms.v1:state/alarms/{alert_key}=<n>`, and a restart as
  a kill or a closed stdin.

`scenarios.md`'s conventions now name `owner --help` as the agreed
interface. Every assumption zk2py's adapter (`RustAlarms`) made by trying
in round 39 is what the help now says, and no answer changed. Driven from
it alone, the owner example passes §1 to §10, 74 of 74.

### F-134 · ambiguity · alarms.v1 0.2 §2.4: "nothing after its value", and whitespace

> §2.4 (0.2): "UTF-8, JSON (RFC 8259) with nothing after its value, no
> duplicate member at any depth, every number finite, and every string with
> a UTF-8 form"

RFC 8259's `JSON-text = ws value ws` allows whitespace after the value, a
trailing newline included. "Nothing after its value" read literally
refuses it. So a put of `{…}\n`, as an owner that writes with a newline
makes one, reads as a document to one reader and as undecodable to the
other.
- `documents.json`'s case "text after the document" has `{}` after the
  value, which both readings refuse.
- No case has whitespace alone after the value.
- Core's D000, which §2.4 cites, says only "strict JSON".

Neither owner puts any whitespace (measured above), so the two
implementations do not differ on each other's puts.

**zk2py:** RFC 8259's reading. Its four whitespace bytes (space, tab,
LF, CR) around the value read, and anything else after it is `json`: a
no-break space, U+2028, a vertical tab, a form feed, a NUL, text, or a
second value. A byte-order mark before it is `json` too.
- `alarms.strict_json` strips exactly those four bytes itself
  (`alarms.JSON_WS`), so the reading does not rest on the JSON library's.
- `just py-conformance`'s `readings` family states it as 11 cases, each
  naming F-134. A later text's fixture would overturn them visibly.

That was made explicit after review of PR #807. Before, the same reading
rested on `jcs.loads`, measured to behave identically.

**Wording:** "nothing after its value but whitespace (RFC 8259's `ws`)",
or "and no whitespace before or after it". Either way, a fixture case
`{…}\n` would pin it.

**Status at alarms.v1 0.3: resolved by 0.3, as zk2py read it.** §2.4:
"nothing after its value but whitespace … Whitespace (0.3) is RFC 8259's
`ws`, before the value and after it … Any other character after the value,
a form feed or a no-break space included, is text after it."
`documents.json`'s four new cases pass unchanged. The `readings` family's
11 cases now cite 0.3, as cases beyond the fixture's: CRLF, VT, U+2028,
NUL, a second value, and a byte-order mark before the value, which zk2py
refuses since RFC 8259's `JSON-text` has none.

**Where zk2py's reading of alarms.v1 0.2 needed a guess, beyond the
findings.**
- **Where the window starts.** §2.15's window "runs from its declaration".
  Core §2.7, which §2.15 cites for lossless and presence, counts a
  window's span "from its subscription's declaration plus its reply
  timeout". zk2py counts from the declaration, as §2.15 says. The 1 s
  differs only at the 900 s margin, where every firing alarm is put twice
  over (§2.7).
- **§10 is slow too.** `scenarios.md`'s conventions name §2 and §6 as the
  slow tier, but §10's step 2 listens 905 s too. zk2py runs it on the
  short-ttl copy beside §2 and §6.
- **§8's archive form.** `scenarios.md` §8 has T GET "the archive's form
  of them". zk2py's stand-in archive (`health.v1`'s §6) slugs every chunk,
  wildcards included, where core §4.4 keeps `*` and `**`. So §8 GETs A's
  concrete archive key. That is zk2py's stand-in's limit, not the text's.

## At alarms.v1 0.3 (#609)

`alarms.v1` 0.3 answers F-133 and F-134, each entry above carrying its
status. It also states round 39's two guesses, against zk2py's reading in
one: a window counts from its declaration plus the reply timeout. And it
states five places the reference's tools settled. The core stays at 0.34,
and no fixture case changed. `documents.json` gains four `text` cases,
which zk2py passed unchanged.

**What zk2py implemented, from the text alone** (`zk2py.alarms`):
- **`AlarmsReader`,** a live reader across owners. It replaces round 39's
  `WindowWatch`:
  - one subscription;
  - its receive path watched, a stall or a closed transport being a loss
    at the instant seen (`budget.StallWatch` now records instants);
  - each owner's instance tokens watched, a gap being a withdrawn token
    or a new instance appearing, an owner joining included;
  - each S4 GET recorded with the instant it was asked.
- **§2.15's window,** at an instant:
  - it runs from the declaration plus the reply timeout, ended a lease and
    the reply timeout before the instant judged;
  - a loss or a gap breaks it until the first complete GET after it,
    reading `window_lost` or `owner_gap` until then;
  - each re-read re-seeds, as the reference's live reader does.
- **§2.14 step 5's held sample** (0.3): a key the establishing complete
  GET did not answer, with nothing of it after that GET was asked, holds
  none, so it reads `not_held`.
- **A one-shot tool's scope** (§2.15, `scenarios.md` §8): `archive_form`
  (core §4.4, wildcards kept, every other chunk slugged) and
  `owners_from_archive`, the owners an archive's keys name.

**`just py-conformance`: 1757 of 1757.** The `alarms` family is 304
checks (`documents.json` 62). `readings` is 19: 11 of §2.4's whitespace,
and, after review of PR #809, six of the window's healing and two of an
incomplete scope.

**`just py-alarms`: 74 of 74** against zk2py's owner and 74 of 74 against
`owner-ac`, both exiting 0. That is round 39's 66, with:
- §3: a reader that heard A firing and missed its resolve and delete reads
  `no, not_held` once the owner's alarms are established by a GET that
  answers B (0.3's step 5);
- §8: two tools, as the reference's §8 runs them (0.3):
  - a live reader, which knows of the owner from step 1;
  - a one-shot reader made at step 3, which knows of it by `lab/archive`'s
    keys.

  Each judges A `not_asked, absent` and counts the owner not asked;
- §10, on `AlarmsReader`: step 2 waits 0.3's span, and four extras follow.
  On each owner:
  - a gap in `lab/b`'s presence (a restart) reads `owner_gap` with no GET
    since;
  - a complete GET after it heals the gap, and the window runs again from
    it (`window_short`);
  - a loss, noted by hand since a zenoh-python callback subscriber has no
    queue to overflow, reads `window_lost`, and A `unobservable,
    incomplete` (step 4);
  - a complete GET after it heals the loss: `answered`, and A `yes,
    firing`.
- §5: its SHOULD is checked by a GET at the token (F-135), the arrival
  order at S printed as an uncounted NOTE.

§2, §6 and §10 ran on the 4 s copy. §2 also ran alone at 900 s on each
owner: 9 of 9, the owner example re-putting after 427.5 s and zk2py's
after 449.8 s. §6 at 900 s takes about 15 minutes, beyond a 10-minute
command, so it was not run at 900 s. Nor was §10's step 2, 0.3's slow
tier, whose window plus watch is 913 s.

### F-135 · gap · alarms.v1 0.3 scenarios.md §5, expected 3: "Before the new instance token appears", observed at a subscriber

> scenarios.md §5, expected 3: "Before the new instance token appears (a
> SHOULD), and before any GET is answered with them (a MUST), S receives:
> A firing, unchanged … a delete of R's key, and of P's key."

The expectation is observed at S, as the order of S's deliveries against
the new instance token's arrival at S's liveliness subscriber. That order
is not the owner's.
- A put of `alarms/{alert_key}` (`interactive_high`, `express`) and a
  liveliness declaration travel as different messages, and S sees no
  order kept between them.
- **Measured:** zk2py's owner puts A, and deletes R's and P's keys, before
  its descriptor and tokens, by construction (`Owner.before_tokens` runs
  before step 3). Yet at S the token arrived first in 2 of 7 runs of §5,
  by 0.04 ms. In the others the put led by 0.06 to 0.99 ms.
- The owner example, which does more between its puts and its token, led
  by 0.7 to 1.5 ms in each of its 7 runs. Its margin is wider, not a
  different order.
- Core and its other profiles observe "before the tokens" another way:
  a GET made the moment the token appears, which must already answer the
  value (presence.md §1; `health.v1` scenarios §1, "Both of T's GETs
  answer the status … It was put before the tokens"; core §8.2, "a GET
  made when the interface token appears then finds it").

**zk2py:** §5 checks the SHOULD as core does. G GETs the moment S sees the
new instance token. The reply must hold:
- A firing, t₀ kept, under the new instance's stamp;
- B under that stamp too;
- a `reply_del` for each of R's and P's keys.

A GET that does not come, or a missing reply, is reported with its
cause. Both owners pass
it in every run. The arrival order at S is printed, not counted.

**Wording:** "3. G GETs the moment S sees the new instance token. Its reply
holds A and B under the new instance's stamps and a `reply_del` for R's and
P's keys (the SHOULD; core §8.2, 'State values')", with S's receipts
listed without an order against the token.

**Status at alarms.v1 0.5: resolved by 0.5, as zk2py checked it.**
`scenarios.md` §5, step 3: "G GETs the moment S sees the new instance
token. Then the owner evaluates its rules once." Expected 3: "G's GET at the
token holds A … and B, unchanged, each under the new instance's stamp id,
and a `reply_del` of R's key and of P's key, stamped by it … S receives, in
no order against the token". §2.9 gains "How 'before its token' is
observed".

zk2py's §5 now takes the GET as a step before the owner's evaluation, and
checks the `reply_del`s' stamps too. Both owners pass.

### F-136 · gap · alarms.v1 0.3 §2.14 steps 4 and 5: a key whose resolve and delete a loss took never reads no

> §2.14 step 4: "**A loss.** A subscription lost deliveries, and no GET has
> answered this key since, with a value or a `reply_del`: **unobservable**,
> `incomplete` … A GET that drew nothing for the key, complete or not, does
> not heal the loss for it (0.2): an empty reply set is no verdict (core
> O5)."

> §2.14 step 5 (0.3): "A sample the reader holds is none (0.3) when a
> complete GET its owner answered, the one that established its alarms,
> did not answer the key, and nothing of the key arrived after that GET was
> asked: the owner answers every live key (core S2), so the key's delete
> went past the tombstone window unheard"

> CHANGELOG 0.3: "Without it, a reader started before a resolve it did not
> hear would hold an alarm firing until it went stale."

§2.14 is "the first of these steps that answers", and step 4 comes before
step 5. Take a reader that holds A firing, whose subscription then lost
deliveries, A's resolve and delete among them.
1. Its re-seed, past the tombstone window, is complete, answers B, and
   draws nothing for A.
2. §2.15 establishes the owner's alarms: the window runs again from that
   GET, and it answered a key.
3. Nothing of A arrives again: it was deleted.

Then:
- step 4 answers A first, **unobservable**, `incomplete`, since no GET has
  answered A "with a value or a `reply_del`";
- none ever will, the key being past the tombstone window;
- so A stays `incomplete` for as long as the reader holds it;
- and "is anything firing at or above *S*?" over that owner, established,
  reads `alarm_unobservable` (§2.15 step 3) for as long, whenever A's last
  severity is at or above *S*. It can never read **no**.

Step 5's 0.3 sentence describes exactly this key ("its delete went past
the tombstone window unheard"), and is never reached. It applies only
where step 4 does not:
- a reader with no subscription, polling by GET;
- a sample held from before the subscription.

The CHANGELOG's case, "a reader started before a resolve it did not hear",
is a reader that lost the resolve. zk2py's model, from the text as
written:
- A reads `unobservable, incomplete`;
- the owner, at warning, reads `alarm_unobservable`;
- with the subscription left out, the same facts read `no, not_held`.

**zk2py:** the text as written: step 4 first, so such a key reads
`incomplete` until the reader forgets it. The text never says when a
reader may forget a key. zk2py's live reader now feeds its own losses into
step 4: a loss marks the subscription's observations lossy, and a GET is
the re-seed only when asked after it. §10's extra checks this on both
owners, A reading `incomplete` after a loss and `yes, firing` once a GET
re-seeds it.

**Wording:** step 4, "…and no GET has answered this key since, with a value
or a `reply_del`, and no complete GET that established its owner's alarms
left it out with nothing of it arriving after that GET was asked (step 5)".
Or state that a reader forgets a key once an establishing GET leaves it
out.

**Status at alarms.v1 0.5: resolved by 0.5, against zk2py's reading.**
§2.14 step 4: "…nor has the GET that established its owner's alarms
(§2.15) left it out (step 5)". Step 5 defines "left the key out" once, "for
an owner established either way (`answered` or `window`)", a delete
included.
- zk2py's judge now reads `Reading.left_out`, which heals step 4 and makes
  the held sample none.
- Its reader computes `left_out` instead of dropping the sample.
- `conclusions.json`'s six new `reading` cases pass.
- §3's extra shows it live, on both owners. A reader that held A firing and
  noted the loss that took A's resolve and delete reads `no, not_held`
  once its re-seed, answering B, establishes the owner.

### F-137 · gap · alarms.v1 0.3 §2.15: the second question over a scope whose read was not whole

> §2.15 (0.2, 0.3): "**The owners in scope** of a selector … are the
> services it selects that the reader knows of: those its presence read
> finds, through instance tokens and descriptors (§2.13), and those it knew
> of before, by an earlier reading of its own or by an archive's keys (core
> S6)."

> §2.15, step 5: "otherwise **no**, `none_firing`", and §5: "**No:** Every
> owner asked in scope established, and none of its alarms reads yes, or
> unobservable, at *S* or above."

The scope is whatever the reads that find owners returned. Those reads can
fall short of whole, and the text says nothing of it:
- a presence read possibly incomplete (core §8.1: "a GET that ran to its
  timeout");
- an archive's GET that ended at its timeout or with an error reply;
- an archive key that does not read back into an alarm's key.

An owner missed that way is in no step of §2.15's list. So the steps read
as written answer "is anything firing?" with **no**, `none_firing`,
whenever the owners that were found are established and quiet. Yet an
owner the read missed may hold a firing alarm. That is the "no" from
silence that core O5 forbids. For one owner, §2.14 step 1 already makes a
presence read possibly incomplete **unobservable**; a scope has no such
step.

**zk2py:** a scope read that is not whole (`ScopeRead.complete` false:
presence or the archive GET incomplete, or `skipped` > 0) adds one
**unobservable** owner, so the answer is `owner_unobservable` unless an
alarm reads yes. Found by review of PR #809, where zk2py's
`owners_from_archive` dropped an unreadable key silently. The `readings`
family has the case.

**Wording:** §2.15, the second question: "an owner in scope is
unobservable, or the reads that found the scope were not whole (a presence
read possibly incomplete, an archive's GET short of its final reply, a key
that does not read back): **unobservable**, `owner_unobservable`" (or a
reason of its own).

**Status at alarms.v1 0.5: resolved by 0.5, partly against zk2py's
reading.**
- §2.15 "Whole": "the reader's latest presence read of them ran to its
  final reply … and, for a reader that follows instance tokens since, it
  lost none of their moves".
- A step of its own: "the reads that found the scope were not whole …
  **unobservable**, `scope_incomplete`". It comes after
  `owner_unobservable` and before an alarm's.
- "An archive's read plays no part in it". An owner known only by an
  archive's keys is absent and stops no answer, so an archive read cut
  short "can leave out only such an owner".

zk2py changes on both points:
- the phantom unobservable owner is gone, replaced by
  `any_firing(scope_whole)`;
- `ScopeRead.whole` is the presence read's alone, and an archive's read
  only sets `archive_short`.

The six new `any_firing` cases pass.

**After review of PR #809** (zk2py's code; F-135 tightened, F-137 added):
- **The window's healing** is a pure function, `window_span`, with six
  cases in `readings`.
  - A GET asked inside a stall never healed it: a stall is seen by the tick
    that ends it, and only a GET asked after that instant heals.
  - Healing is now strictly after. A GET asked at the very instant a break
    was seen heals nothing.
  - A stall still open when the reader judges is now a loss too (core
    §2.7, "the one still open when it is read included").
- **The window's start** (`add_owner`): the presence read and the token
  events share one lock, and the subscription delivers no history, so a
  token already held is never a gap. A start-of-window presence read
  possibly incomplete does not show "held its instance token from the
  window's start". It reads `owner_gap` until a complete GET after that
  read. A complete, empty read is an absent owner, whose joining is a gap.

**Where zk2py's reading of alarms.v1 0.3 needed a guess, beyond the
findings.**
- **"From that GET."** The window "runs again from the first complete GET
  that followed" a loss or a gap, and "counts … from that GET". zk2py
  dates a GET by the instant it was asked, as step 5's 0.3 sentence dates
  one ("after that GET was asked"). Its completion is at most the reply
  timeout later. Only the margin feels it, as with round 39's "where the
  window starts".
- **Which archive.** A one-shot tool "knows of it by the archive's keys",
  and the text does not say how the tool finds the archive. Core §4.4's
  archive is a service implementing `archive.v1`, which a tool could find
  by presence. zk2py's stand-in archive holds no presence, so §8 names
  `lab/archive` to the tool, as the deployment's word.
- **The horizon judged at.** §2.7 says the contract's; §2.14 and §2.15 say
  900 s. On a short-ttl copy the two differ, and the reference judges at
  900 s whatever revision (0.3, "Deliberately not stated"). zk2py judges at
  the horizon of the revision the owner implements, §2.7's MUST. That is
  why its §6 and §10 can run on the 4 s copy. No finding: 0.3 names the
  question and defers it to a later minor.
- **A held delete.** Step 5's 0.3 rule makes "a sample the reader holds"
  none. zk2py applies it to a held delete too, which then reads
  `not_held` rather than `deleted`. Both are **no**, and both are
  `not_firing` for the last question.

## At alarms.v1 0.5 (#609)

Two texts, read cold together, with the core unchanged at 0.34:
- **0.4** adds §2.16, "Reporting changes", which the reference notifier
  settled. It changes no fixture.
- **0.5** answers F-135 to F-137, each entry above carrying its status. It
  states two of round 40's guesses, both as zk2py read them:
  - "A GET is dated by the instant it was asked, its replies included";
  - a held delete reads `not_held`.

`conclusions.json` goes from 95 cases to 107. Every `reading` case gains
`left_out`, every `any_firing` case `scope_whole`, and six cases each are
new for F-136 and F-137. The owner black box is still 0.3's, `owner-ac`.

**What zk2py implemented, from the texts alone:**
- **§2.14 steps 4 and 5 (0.5):** `Reading.left_out`, which heals step 4
  and makes the held sample none, so the key reads `not_held`.
  `read_owner` computes it, and no longer drops the sample.
- **§2.15 "Whole" (0.5):**
  - `any_firing(scope_whole)`, with step 3 `scope_incomplete`;
  - `ScopeRead.whole`, the presence read's alone, with `archive_short`
    for an archive's read, which plays no part;
  - `AlarmsReader.scope_read`, not whole after a loss seen since its
    presence read began.
- **§2.16 (0.4):** `ChangeReporter`. It reports §2.14's answer after each
  change, never the change's name alone. A change is the answer moving
  between yes, no and neither, or, while yes, the severity moving. A loss
  is reported once for the scope (the MAY), and each key again once
  healed (F-139). An alarm firing at the first reading is reported (the
  SHOULD).

**`just py-conformance`: 1844 of 1844.**
- At the start of the round, 7 of `conclusions.json`'s new cases failed:
  the three F-136 cases whose `left_out` makes the held sample none (one
  healing step 4 after a loss, two without a loss), and four of the F-137
  cases.
- The `alarms` family is 316. `readings` is 94:
  - §2.4's whitespace, 11;
  - the window's healing, 6;
  - 0.5's scope, 2;
  - §2.16's reporter, 75: one per `transitions.json` case (27), one per
    `conclusions.json` reading (47), and a loss sequence.

**`just py-alarms`: 80 of 80** against zk2py's owner and 80 of 80 against
`owner-ac`, both exiting 0. That is round 40's 74, with:
- §3's extra now losing the resolve, which reads `no, not_held` (0.5);
- §5's GET at the token taken before the owner's evaluation, its
  `reply_del`s stamped by the new instance (0.5);
- §8 and §10 reading the scope as 0.5 says;
- §10 running the change reporter live. It reports:
  - the first reading's firing alarm, with its answer;
  - nothing for A's confirmations;
  - B's raise at its severity, and B's resolve and delete as **no**;
  - a loss once for the scope, and no key for it;
  - nothing when the healed A still fires at its severity.

§2, §4, §6 and §10 ran on the 4 s copy. §10's step 2 at 900 s stayed
out, as before.

### F-138 · gap · alarms.v1 0.5 conformance/conclusions.json: `left_out` and `scope_whole` undescribed

> core Appendix E: "Each fixture file states its own format in its
> `description` member."

> CHANGELOG 0.5: "`conclusions.json` gains the two inputs in every case it
> reads them in"

`conclusions.json`'s description is 0.3's, and names neither input.
- **`scope_whole`** reads plainly from §2.15's "Whole".
- **`left_out`** is wider in the fixture than the text's term.
  - §2.14 step 5 defines "left the key out" for "the GET that established
    its owner's alarms" only.
  - Yet two cases give `left_out: true` for an owner whose alarms are not
    established (`held_whole: false`): "a GET after it that left the key
    out but did not establish its owner's alarms", and "the GET that left
    the key out was asked before the loss".
  - Both expect `incomplete`. So in the fixture, `left_out` means a GET
    left the key out, and the judge counts it only with `held_whole`.
  - An implementation reading `left_out` as the text's term, "the
    establishing GET left it out", without that gate, reads those two
    cases `no, not_held`.

**zk2py:** `left_out` counts only with `held_whole` (`judge`: `left_out =
r.left_out and r.held_whole`), which passes all six. That reading came from
the cases' notes, not from a stated format.

**Wording:** the description: "`left_out`: a GET, the latest after any
loss, answered the key with neither a value nor a `reply_del`, and nothing
of the key arrived after it was asked; it is §2.14 step 5's left-out key
only when that GET established the owner's alarms (`held_whole`).
`any_firing` … `scope_whole`: whether the reads that found the scope are
whole (§2.15)."

**Status at alarms.v1 0.6: resolved by 0.6, as zk2py read it.** The
description now states both inputs.
- `left_out`: "the reader's latest GET of the owner answered the key with
  neither a value nor a `reply_del`, and nothing of the key arrived after
  it was asked; it is §2.14 step 5's key left out, healing step 4's loss
  for it, only with `held_whole`; without it, it changes no answer".
- `scope_whole`: "whether the reads that found the owners in scope are
  whole, §2.15: the latest presence read complete and nothing lost since;
  an archive's read plays no part".

Read from the description alone, every one of the 107 cases follows, the
two with `left_out` and no `held_whole` among them. zk2py's gate stands.

### F-139 · ambiguity · alarms.v1 0.4 §2.16: "reports each key again" after a loss

> §2.16: "**A loss is the reader's.** … A tool MAY report it once for its
> scope rather than once per key, if it then reports no key's answer the
> loss made unobservable, and reports each key again as the GET that heals
> the loss answers it, or, establishing its owner's alarms, leaves it out"

> §2.16: "**What is a change to report:** the answer moving between yes,
> no and neither; and, while yes, the document's severity moving"

"Reports each key again" reads two ways:
1. **Every key is reported anew** once healed, whether or not its answer
   moved. An alarm firing before the loss and firing after it is reported
   firing a second time.
2. **Reporting resumes for each key** as healed, against its last report.
   A key whose answer did not move is not reported, as "What is a change to
   report" says.

For a notifier the two differ by one page per firing alarm after every
loss. The CHANGELOG 0.4 suggests reading 2: "each key keeps its last report
until the GET that heals the loss answers it". The text does not say it.

**zk2py:** reading 2. `ChangeReporter` keeps each key's last report through
the loss and reports a healed key only if its answer moved. `readings`
checks it (a loss, A still firing, B now `no`: B alone is reported), and so
does §10 live.

**Wording:** "…and reports each key again, against its last report, as the
GET that heals the loss answers it", or "…reports each key's answer again,
moved or not".

**After review of PR #812** (zk2py's code; no finding changed):
- **`left_out` and a `reply_del`.** A key the owner answered with a
  `reply_del` was never left out: the GET's replies keep every reply, so it
  reads `deleted`. The rule is now a pure function, `is_left_out`, with six
  cases in `readings`, the `reply_del` among them.
- **The loss instants** of `note_loss` and of the stall watch were already
  on one monotonic clock, `scope_read`'s too. Unchanged.
- **`ChangeReporter`'s loss** is now tracked per key:
  - one scope report when the first key goes lossy;
  - none for a second loss while any key is unhealed;
  - an "observable again" report when the last one heals, after which a
    later loss is reported again.

  A partial update (one owner's keys) heals only its own keys. In an update
  of the whole scope, a key known before that is missing is reported once,
  `unobservable`/`not_read`, never dropped silently. Four loss sequences
  pin these cases, and §10's live check now expects "observable again".
- A `conclusions.json` case missing `left_out` or `scope_whole` fails alone.
  §5's GET at the token, if it does not come within 5 s, is a failed check
  with its cause.

`just py-conformance`: 1853 of 1853 (`readings` 103). `just py-alarms`: 80
of 80 on each owner.

**Status at alarms.v1 0.6: resolved by 0.6, as zk2py read it.** §2.16:
"Each key then keeps its last report through the loss … the key's answer
is a change to report only if it moved from that last report … an alarm
firing before the loss and after it, at one severity, is not reported
again." That is reading 2, `ChangeReporter`'s. 0.6 also made zk2py see
that its baseline must be the last *report*, not the last answer read:
see "At alarms.v1 0.6".

**Where zk2py's reading of alarms.v1 0.4 and 0.5 needed a guess, beyond
the findings.**
- **A key first read after the first reading** at **no** or **neither** (a
  late resolve, say) is recorded, not reported. Nothing moved from a report
  never made. A key first read **yes** is reported, as at the first
  reading.
- **"Lost none of their moves since"** (§2.15 "Whole"): zk2py counts a loss
  seen from the instant the presence read began.
- **The session cut off from every router** (0.4, "Deliberately not
  stated"). zenoh answers its reads at once, complete and empty. zk2py's
  reader does not guard against it: such a reader would read every owner
  absent and heal a loss. The text defers it to the core.

## At alarms.v1 0.6 (#609)

0.6 answers F-138 and F-139 (each entry above carries its status), and
states two of round 41's guesses. The core stays at 0.34, and no fixture
case changed: only `conclusions.json`'s description did, and zk2py
passed it unchanged.

**zk2py against 0.6's two new rules.**
- **§2.15 "Whole": a presence read is dated by the instant it was
  asked,** and "a loss heard at or after that instant counts against it,
  wherever its replies arrive". zk2py already counted a loss from the
  read's start. But `scope_read` decided wholeness when the read returned,
  so a loss heard during the GETs that followed it, before the second
  question was asked, was missed. Now:
  - `ScopeRead` carries the read's ask;
  - `AlarmsReader.scope_whole` decides at the instant the question is
    asked, through a pure `presence_read_whole`;
  - `readings` has four cases, among them a loss at the ask's very instant.
- **§2.16, "A key's first answer … moves from no report":** yes is
  reported, no is not, a neither of the key's own MAY be, and a neither of
  the reader's own sight is not.
  - zk2py reported no first neither, one of the two readings the MAY
    allows.
  - But its `ChangeReporter` compared each answer with the key's last
    *recorded* answer, not its last *report*. So a key first read `no`
    (recorded, not reported) and then `unobservable, clock_untrusted` was
    reported. That is a neither of the reader's sight, for a key with no
    report: "no answer to report".
  - It now compares with the last report. A key with no report keeps none
    until it reads yes, and a key gone from the whole scope is reported
    only if its last report was yes or no.
  - `readings` has four first-answer cases. One of them is that sequence,
    which round 41's reporter fails.
  - F-139's answer, "each key then keeps its last report through the
    loss", is the same baseline.

**`just py-conformance`: 1861 of 1861.** The `alarms` family stays at 316,
and `readings` is 111: round 41's 103, with four first-answer and four
presence-dating cases.

**`just py-alarms`: 80 of 80** against zk2py's owner and 80 of 80 against
`owner-ac`, both exiting 0. These are round 41's checks: the reporter's
live checks in §10 read the same under the new baseline, and §10 asks for
wholeness at the question. §2, §4, §6 and §10 ran on the 4 s copy, with
§10's step 2 at 900 s left out as before.

### F-140 · gap · alarms.v1 0.6 §2.16 "A key's first answer": five of §2.14's reasons are on neither list

> §2.16 (0.6): "Neither, for a cause of the key's own (stale, undecodable,
> not asked), MAY be reported. Neither that only the reader's own sight
> makes so (a loss no GET has healed, a clock not yet trusted, a reading
> too near the horizon to tell, a window not yet listened, nothing read
> yet, presence or a descriptor not read whole) is no answer to report"

> §2.14 step 6: "unobservable: **unobservable**, with `freshness.v1`'s
> reason"

The two lists cover the reasons as follows:
- **The key's own:** `stale` and `undecodable`, and not asked (`absent`,
  `not_listed`, `last_known`, `previous_instance`).
- **The reader's own sight:** `incomplete`, `clock_untrusted`,
  `near_horizon`, `listened_too_short`, nothing read (`not_read`,
  `no_observation`, `silent`), and `presence_incomplete` and
  `no_descriptor`.
- **On neither list,** each a first answer the first question can give:
  - `face_closed` (§2.14 step 1). The deployment closes the face, so it
    is not the key's own, and no read settles it.
  - `nothing_crossed` (step 5 or 6). The link may be out, which is the
    reader's sight, or the alarms may never cross.
  - `no_stamp` (freshness.v1 §2.6). The owner's reply breaks S2, which
    looks like the key's own.
  - `clock_disagrees` (freshness.v1 §2.6). The stamp is ahead of the
    reader's clock beyond the delta, and either clock may be the one
    ahead.
  - `not_a_horizon` (freshness.v1 §2.1), with a revision whose
    `freshness.ttl_s` is no horizon. That is the contract's, not the key's
    or the reader's.

A tool that takes the MAY, reporting a first neither of the key's own,
must classify these five, and two tools can classify them apart.

**zk2py:** unaffected. Its reporter takes the other branch of the MAY and
reports no first neither, so it never needs the classification.

**Wording:** name these five in one list or the other, or close the lists:
"any other neither is the reader's own sight".

**After review of PR #814** (zk2py's code; no finding changed):
- **A key gone from the whole scope.**
  - A key with no report is forgotten, and a reappearance is a first
    answer. That is §2.16's "moves from no report", and it holds for a
    first neither of the key's own that zk2py chose not to report too.
  - A key with a report now keeps it. Before, one last reported yes or no
    was reported gone (`unobservable`, `not_read`) and then forgotten, so
    a reappearance at **no** read as a first answer and was not reported.
    That left the gone report standing, though a report then existed for
    the no to clear. A key last reported neither had the same fault.
  - `readings` has four cases. The old reporter fails the two where a key
    had a report.
- **A presence read with no ask** is not whole. `owners_by_presence` dates
  every read it makes, `scope_read` takes that ask, and §10 asserts its
  scope read was made before it uses it.

`just py-conformance`: 1866 of 1866 (`readings` 116). `just py-alarms`: 80
of 80 on each owner.

**Status at alarms.v1 0.7: resolved by 0.7.** §2.16: "Neither splits by
its reason, in two closed lists".
- **The reader's own sight**, nine tokens: `incomplete`,
  `presence_incomplete`, `no_descriptor`, `not_read`, `nothing_crossed`,
  `clock_untrusted`, `listened_too_short`, `near_horizon`,
  `no_observation`.
- **"Any other reason"**, which names `face_closed`, `no_stamp`,
  `clock_disagrees` and `not_a_horizon` among them.

Of the five this finding named:
- `nothing_crossed` is the reader's sight;
- `face_closed`, `no_stamp`, `clock_disagrees` and `not_a_horizon` are on
  the other list.

zk2py's reporter, on the branch of the MAY that reports no first neither,
meets the text unchanged. One reason the text names on neither list
remains: F-141.

**Where zk2py's reading of alarms.v1 0.6 needed a guess, beyond the
findings:** none.

## At alarms.v1 0.7 (#609)

0.7 answers F-140: §2.16's first answer of neither "splits by its reason,
in two closed lists". There is no model or fixture change, and the core
stays at 0.34.

**Is every reason the first question can give on exactly one list?** Yes,
by construction: the second list is "any other reason". zk2py checked it
concretely, driving its judge over one reading for each of the twenty
neither reasons §2.14 can give (`readings`):
- **The reader's own sight (9):** `incomplete`, `presence_incomplete`,
  `no_descriptor`, `not_read`, `nothing_crossed`, `clock_untrusted`,
  `listened_too_short`, `near_horizon`, `no_observation`.
- **Any other reason (11):** `absent`, `not_listed`, `last_known`,
  `previous_instance`, `face_closed`, `undecodable`, `stale`, `no_stamp`,
  `clock_disagrees`, `not_a_horizon`, and `silent`.

Ten of the eleven fit the second list's criterion, "something of the
alarm, its owner or the deployment that time does not settle". `silent`
does not (F-141).

**zk2py's reporter meets 0.7** on the branch of the MAY it takes. It
reports no first neither, which the reader's own sight requires and the
other list allows. No code changed. `READER_SIGHT` and
`first_neither_list` exist only for the check.

**`just py-conformance`: 1867 of 1867** at first. The `alarms` family
stays at 316, and `readings` is 117 with the classification case.

**After review of PR #816:** the classification is now 21 checks. Each of
the twenty readings is checked on its own: its answer is neither yes nor
no, its reason is the one intended, and its list is the one 0.7 gives it.
A last check ties the table to `READER_SIGHT`. Before, the case keyed what
it found by the reason the judge returned and dropped the intended one, so
a reading that drifted to another reason could pass. `not_a_horizon` is in
the table, with its horizon. `silent` stays on "any other reason", 0.7's
classification. `just py-conformance`: 1887 of 1887 (`readings` 137).

**`just py-alarms`: 80 of 80** against zk2py's owner and 80 of 80 against
`owner-ac`, both exiting 0. §2, §4, §6 and §10 ran on the 4 s copy, with
§10's step 2 at 900 s left out as before.

### F-141 · gap · alarms.v1 0.7 §2.16 "A key's first answer": `silent` is the reader's own sight, but on neither named list

> §2.16 (0.7): "**The reader's own sight**, which time or the next read
> settles: … nothing read yet, here or across a face (`not_read`,
> `nothing_crossed`); and, from `freshness.v1`, a clock not yet trusted …
> and no observation to judge (`no_observation`)." "**Any other reason**
> says something of the alarm, its owner or the deployment that time does
> not settle … It MAY be reported."

> §2.14 step 6: "unobservable … **unobservable**, with `freshness.v1`'s
> reason"; `freshness.v1` §2.6: no reply for the member, `silent` (core O5,
> S6).

The first question gives `unobservable`, `silent` at step 6 when the
reader holds a sample of the key and its only observation is a GET that
drew nothing for it. That happens, for instance, to a reader that reads
by GET alone. Its earlier GET answered the key. Its latest drew nothing
for it, and did not establish the owner's alarms (an empty reply set, no
window), so step 5's left-out key does not apply. zk2py's judge gives it
(`readings`).
- `silent` is silence, "no verdict" (core O5), and the next read settles
  it. That is the reader's own sight by the list's own words, a sibling of
  `not_read` and `no_observation`.
- The closed reader's-sight list does not name it, so it falls to "any
  other reason".
- A tool that takes the MAY would report a first `silent`, and so report a
  key on nothing but its own silence, which §2.16 keeps out for every
  other "nothing read yet".

**zk2py:** unaffected. Its reporter reports no first neither.

**Wording:** name `silent` on the reader's-sight list ("nothing read yet,
here or across a face, or a GET that drew nothing for a key held (`not_read`,
`nothing_crossed`, `silent`)"). Or, if the text means it as the owner's, say
so.

**Status at alarms.v1 0.8: resolved by 0.8, as zk2py read it.** §2.16's
reader's-sight list now names "a GET that drew nothing for the key
(`silent`, an empty reply set being no verdict, core O5; 0.8)", so the list
is ten. zk2py's classification follows: the `silent` row expects the
sight list, and the tie check 0.8's ten. Each of the twenty neither reasons
the first question gives is on the list its own criterion names.

**The final whole-text pass over alarms.v1 0.7.** This round re-read §0 to
§5 and the appendices against zk2py's two halves.
- **F-141 above** is the one thing still left open.
- **Considered and not filed:** a resolved document's other members. §2.5's
  table fixes a resolve's `state`, `timestamp` and `observed_at_ms`, not its
  `severity`, `summary` or `category`. Both owners keep them as held, and
  no reader rule reads them on a resolved document: §2.14 and §2.15 answer
  no, and §2.16 reports no with no severity. So no answer turns on them.
- **Deferred by the text itself, and not findings:**
  - the horizon a reader judges at (0.3);
  - a read through a session cut off from every router (0.4);
  - which branch of the MAY (0.7).

Beyond F-141, no rule zk2py met needed a guess.

## At alarms.v1 0.8 (#609)

0.8 answers F-141 by putting `silent` on §2.16's reader's-sight list, which
is now ten tokens. It considers and keeps a resolved document's
`severity`, `summary` and `category` unfixed, the point round 43's pass
noted and did not file. There is no model or fixture change, and the core
stays at 0.34.

**zk2py:** `READER_SIGHT` and the classification table follow 0.8. Twenty
checks, each a reading with its reason, answer and list, and a tie check
that the sight list is 0.8's ten. Its reporter reports no first neither,
so it meets §2.16 on that branch of the MAY with no change.

**`just py-conformance`: 1887 of 1887** (`alarms` 316, `readings` 137).

**`just py-alarms`: 80 of 80** against zk2py's owner and 80 of 80 against
`owner-ac`, both exiting 0. §2, §4, §6 and §10 ran on the 4 s copy, with
§10's step 2 at 900 s left out as before.

**None open.**

**The closing whole-text pass over alarms.v1 0.8.** This round re-read the
text against zk2py's two halves once more, the reader's and the tool's
rules in full: §2.6, §2.11 to §2.16, and §5's table.
- §2.16's lists are closed and consistent. Each of the twenty neither
  reasons the first question gives is on one list, and on the one its
  criterion names.
- Its other bullets are what `ChangeReporter` does:
  - an earlier instance's put is never a firing to report;
  - a resolve after a loss waits for the healing GET;
  - each key keeps its last report through a loss;
  - a key's first answer moves from no report.
- §2.11's face closed to the alarms answers both questions unobservable,
  never no, as zk2py's judge and `any_firing` do.

Nothing a second implementation would have to guess remains. What the text
defers stays deferred by its own word, and none is a finding:
- the horizon a reader judges at (0.3);
- a read through a session cut off from every router (0.4);
- which branch of the MAY (0.7);
- a resolved document's other members (0.8).

## At telemetry.v1 0.1 (#609)

`telemetry.v1` 0.1 is the seventh profile (`spec/profiles/telemetry/`). It
has no standard contract: its first point is a standard schema,
`telemetry.v1.json`, whose types are the profile's by their artifact's id
(core 0.35, §10 point 1). zk2py read it cold with core 0.35 and `link.v1`
0.8. `zk2py.telemetry` is the reader's and the tool's half, from the text
alone:
- the published ids;
- a resource's declaration and its carrier;
- a sample's conformance, with the member's kind and unit;
- a pair's epoch, continuity and rate.

`zk2py.telemetry_scenarios` runs `scenarios.md` §1 to §8 against a zk2py
owner of `meter.v1`.

**Core 0.35** publishes `telemetry`'s four keys in Appendix D. zk2py moved
them from its interim table to its published one. W105 reads them as
before, so no contract's codes changed. §10 point 1's standard schema
needed nothing of the contract reader: a contract lists the file as any
file, and only the profile's reader compares ids. **`link.v1` 0.8** changed
wording only, and zk2py's link module reads no `telemetry.*` key.

**`just py-conformance`: 2166 of 2166.** At the start of the round, 1887
passed and 1 failed: `layout`, since the profile's `conformance/` was
unread.

| Family | Before | After |
|---|---|---|
| layout | 19 of 20 | 20 of 20 |
| telemetry | (none) | 243 of 243 |
| readings | 137 of 137 | 172 of 172 |
| the 19 others | 1731 of 1731 | 1731 of 1731 |

The `telemetry` family covers four files, every case passing the first
time it ran:
- `schema.json`: its 9 cases, and three checks of its header against
  zk2py's tables: the current id, the published ids in order, and the four
  types with their first revisions;
- `declarations.json`: 87;
- `samples.json`: 101;
- `continuity.json`: 43, each rate equal to the fixture's bit for bit.

`examples` stays at 108. The four ZenSight contracts' revision 1.1 list
the profile's file by `../../../spec/profiles/telemetry/telemetry.v1.json`.
Each loads with no finding, builds its published bundle byte for byte, and
is compatible with both its revisions.

Of the 35 new `readings`, 21 read Appendix B over the adopters' real contracts,
which the fixtures do not. They check zk2py's contract reader, which finds
each type's root by following its `$ref`s, where the fixtures give the
roots already followed:
- zenoh-modem: 73 declared metric streams, 23 with a unit, and
  `sdu/dropped_total` a bare counter with no epoch;
- each ZenSight contract, at revision 1.1 and in both published bundles:
  a Point declaring no kind, and netring's two units. The first
  revision's bundle carries the pre-profile id;
- tcgui: no resource this profile's;
- three of §2.3's copies;
- two of F-148.

The other 14 read F-149's `null` members (review of PR #821): a Point's
`value` written `null` is `not_a_point`, a Value's own `value` written
`null` is `wrong_type` for each of the six kinds, so is a `null` Histogram
member, and `epoch`, `unit` and `labels` read as F-149 says.

**`just py-telemetry`: 48 of 48**, exiting 0, in about a minute:

| § | Checks |
|---|---|
| §1 | 10 |
| §2 | 6 |
| §3 | 6 |
| §4 | 5 |
| §5 | 5 |
| §6 | 3 |
| §7 | 5, F-147's reading included |
| §8 | 8, run twice |

§8's two runs:
- **R0 alone**, as the reference runtime runs it;
- **R0F**, behind a second router whose access control denies
  `zk2/*/*/@zk/**` on its `unixsock-stream` face, in every message and
  both flows. A control session on the same router, off the face, sees
  every token that R0F does not. Both read the same pairs.

The Rust owner example's `--telemetry` arrives with #819 TB, so this run
is zk2py's owner alone. zk2py's owner gained `new_epoch`, core §8.1's
make-before-break re-mint in one session. §2 checks its overlap: R sees
I2's token before I1's is withdrawn. The owner's `publish` also gained an
attachment.

**Passing is not deriving.** Two expectations come from the fixture, not
the text:
- the no-rate reason `kind` for a counter paired with another kind
  (F-146);
- `epoch: "point"` on a declaration whose Point is the pre-profile
  revision's, which has no `epoch` member (`declarations.json` case 15).
  The text's "A Point carries an in-band epoch in its `epoch` member"
  (§2.4) agrees once that revision's Point counts as the Point (§3).

### F-142 · gap · telemetry.v1 0.1 §2.11 "The elapsed time": a stamp's nanoseconds

> §2.11: "The elapsed time `e` is the later time minus the earlier, an
> integer of nanoseconds: a Point's milliseconds times 10^6, a stamp's time
> in nanoseconds as zenoh reads it, a receipt in the reader's nanoseconds."

> §2.11: "Two readers that follow it agree bit for bit."

> core Appendix B: "Its time is an NTP64 value, whose low 32 bits are a
> fraction of a second, so its unit is 2^−32 s."

A stamp's time is a whole number of 2^−32 s units. Most such values are no
whole number of nanoseconds, so "in nanoseconds" is a conversion, and "as
zenoh reads it" names one without stating it. Two readers can follow the
text and still compute different values of `e`:
- **Per stamp, then subtract:** convert each stamp with zenoh's own
  conversion, then subtract. zenoh-python's `NTP64.as_nanos()` gave 1 ns
  above the floor of the exact value on each of 2,000 HLC stamps measured
  here.
- **Subtract, then convert:** take the difference of the two NTP64 values
  and convert it once, by floor or by rounding.

Measured over 1,999 consecutive HLC stamp pairs on one session, the first
way and the second (by floor) gave different `e` in about half the pairs:
950 to 1,018 per run. A 1 ns change in `e` changes the rate's double, so the two readers
disagree in its last bits.

`continuity.json` gives `stamp_ns` already in nanoseconds (cases 26 and
27), so no fixture decides. The Point's milliseconds and a receipt's
nanoseconds are exact, and do not have the problem.

**zk2py:** the first way, zenoh-python's `as_nanos()` per stamp
(`freshness.stamp_ns_of`, as every zk2py reader of a stamp already does),
then the difference. `just py-telemetry` prints the measurement with every
run.

**Wording that would remove the guess:** §2.11, "a stamp's time: the later
stamp's NTP64 value minus the earlier's, times 10^9, divided by 2^32,
rounded toward zero". A `continuity.json` case should give two raw NTP64
values whose two conversions differ.

**Status at telemetry.v1 0.2: resolved by 0.2, as zk2py read it.**
§2.11: "its NTP64 seconds, the high 32 bits, times 10^9, plus its 32-bit
fraction times 10^9 divided by 2^32 and rounded up (uhlc's
`NTP64::as_nanos`, which zenoh-python wraps). Each stamp is converted
alone". zk2py states it as `telemetry.ntp64_ns`, which equals
zenoh-python's `as_nanos()` on every one of 2,000 HLC stamps per run of
`just py-telemetry` (a counted check now), and reads `continuity.json`'s
new `stamp_ntp64` through it (cases 43 and 44).

### F-143 · gap · telemetry.v1 0.1 §2.6 "The data item": a JSON payload's duplicate member, a number past the double range, a lone surrogate

> §2.6: "A sample is read as its decoded data item: JSON's data model for a
> JSON resource (RFC 8259) …"

> §2.6: "A float (JSON text with a fraction or an exponent, a CBOR float) is
> the IEEE 754 double it denotes, the nearest one for JSON text."

> §2.6: "A sample the core could not decode (bytes that are not its JSON or
> its CBOR) is unobservable here (`undecodable`)"

RFC 8259 leaves three things to the parser:
- whether names repeat: they SHOULD be unique (its §4), and behaviour
  otherwise is unpredictable;
- the range of numbers (its §6);
- what an escaped lone surrogate means (its §8.2).

So the data item of each case below is the parser's choice:
- **A duplicate member.** A Point with two `timestamp`s, two `value`s or
  two `epoch`s. A parser may keep the first, keep the last, or refuse.
  That gives two rates, two kinds, or two epochs (a reset or not), or
  `undecodable`.
- **`1e400`.** For "the nearest one", the nearest double by distance is
  the largest finite one, so a gauge conforms. IEEE 754's
  round-to-nearest gives +Infinity, which is `not_finite`. A parser that
  refuses it makes the sample `undecodable`. A counter likewise reads
  `out_of_range`, `wrong_type` or `undecodable`.
- **An integer's text beyond the double range** in a gauge (10^400 written
  out). "An integer is read exactly", so it is a finite number and
  conforms. A reader that holds numbers as doubles reads +Infinity.
- **A lone surrogate escape** (`"\ud800"`) in an epoch or a unit. As a
  code point it is one character, a valid epoch. As text it has no UTF-8
  form.

`alarms.v1` 0.2 settled the same questions for its payloads ("Strict JSON
first", F-126). The core states strict JSON for its own documents only
(D000, E029). `samples.json` gives each sample already decoded, so it has
none of these cases.

**zk2py** (`telemetry.decode_json`):
- **The members:** strict JSON's rule, as `alarms.v1` §2.4 reads its
  payloads. A duplicate member, or a string with no UTF-8 form, makes the
  sample `undecodable`.
- **The numbers:** RFC 8259's text, read by §2.6. `1e400` is IEEE 754's
  +Infinity, so a gauge is `not_finite` and a counter `wrong_type`. An
  integer's text is exact, so a gauge of 10^400 conforms. A bound of
  10^400 is read as its double (§2.2), +Infinity, and is `not_finite`.

**Wording:** §2.6, "A JSON sample is read as `alarms.v1` §2.4 reads a
payload: strict JSON (no duplicate member, every string with a UTF-8
form); a number past the double range is +Infinity (or: undecodable)."
The fixture format would need raw text for such a case.

**Status at telemetry.v1 0.2: resolved by 0.2, partly against zk2py's
reading.** §2.6 "Strict JSON": "every number finite … `1e400` is, and so
is an integer's text past the double's finite range (10^400 written out)"
undecodable. zk2py had read `1e400` as +Infinity (`not_finite`) and
10^400 as a conforming gauge; `telemetry.decode_json` now refuses both.
Its strict members (a duplicate, a lone surrogate) stand. `samples.json`'s
eight `json_text` cases pass. One edge of "past the double's finite range"
is left: F-150.

### F-144 · ambiguity · telemetry.v1 0.1 §2.6 "An integer is read exactly": a CBOR bignum, and tags

> §2.6: "**An integer is read exactly:** JSON text with no fraction and no
> exponent, CBOR's major types 0 and 1."

> `samples.json`: "`{"$int": "<decimal>"}` an integer outside -2^63 to
> 2^64-1"; case 2, "2^64, an integer by its text: out of range"

CBOR's major types 0 and 1 hold −2^64 to 2^64−1. So in a CBOR resource,
2^64 can only be a bignum: tag 2 around a byte string (RFC 8949 §3.4.3).
RFC 8949 defines tags 2 and 3 as integers beyond the major types. Two
readings give two reasons:
- **By §2.6's list,** a bignum is no integer, but a tagged byte string. A
  counter of 2^64 is then `wrong_type`, and a histogram whose `count` is a
  bignum is no Histogram.
- **By RFC 8949,** it is the integer 2^64, `out_of_range`, which is what
  the fixture's `$int` gives for the same value read from JSON text.

Both are `does_not`, so no conformance answer changes, but the reasons
differ. The text says nothing of other tags either: a `timestamp` under
tag 1 (epoch time), or a decimal fraction (tag 4) in a gauge. Core §5.2
reads the error envelope's tags as their content ("A tag decodes as its
content"), but that rule is the envelope's.

**zk2py:** core §5.2's habit. A tag is read as its content (`zk2py.cbor`),
so a bignum is a byte string: `wrong_type` for a counter, and `not_a_point`
for a timestamp. Tag 1 around an integer timestamp reads as that integer.

**Wording:** §2.6, "CBOR's major types 0 and 1, and its bignums (tags 2
and 3)", or "… and nothing else: a tagged item is read as its content".

**Status at telemetry.v1 0.2: resolved by 0.2, against zk2py's
reading.** §2.6 "CBOR": "a bignum, tag 2 or 3 around a byte string of any
length, definite or not, read as the integer it denotes … Every other tag
… is read as its content". zk2py had read a bignum as its content, a byte
string. `zk2py.cbor.loads(…, bignums=True)` now reads tags 2 and 3 around
a byte string as integers; the error envelope's reading (core §5.2) keeps
the default. The tag-1 timestamp, read as its integer, stands.
`samples.json`'s seven `cbor_hex` cases pass.

### F-145 · ambiguity · telemetry.v1 0.1 §2.6 checks 8 to 12: which check "needs" bounds an earlier check found wrong

> §2.6: "A reader reports every one that applies, in this order, each
> check made alone, and a check that needs a value an earlier check found
> wrong skipped"

> §2.6: "10. **`bounds_differ`:** a histogram's bounds, valid, … 11.
> **`counts_length`:** `counts` not one longer than `buckets`. 12.
> **`count_sum`:** `count`, and every one of `counts`, in range, and
> `count` not their sum"

Checks 10 and 12 say what they need. Check 11 does not. When the bounds
are not finite (8) or invalid (9), `counts_length` reads `buckets`, which
an earlier check found wrong, but it needs only its length. Two readings
give two answers:
- `buckets: []`, `counts: [5, 1]`, `count: 6` is `[bounds_invalid]`, or
  `[bounds_invalid, counts_length]`;
- `buckets: [0.005, Infinity]` with four counts is `[not_finite]`, or
  `[not_finite, counts_length]`.

`samples.json`'s cases 49 (an infinite bound) and 51 (no bounds) both have
`counts` one longer than `buckets`, so neither decides.

**zk2py:** `counts_length` is checked whenever the value is a Histogram
(check 6). It needs the two lengths alone, and neither is what checks 8
and 9 found wrong. `count_sum` is checked by its own stated precondition,
whatever `counts_length` found.

**Wording:** §2.6 check 11, "… checked whenever the value is a Histogram",
and a `samples.json` case with no bounds and two counts.

**Status at telemetry.v1 0.2: resolved by 0.2, as zk2py read it.** §2.6
check 11: "checked whenever the value is a Histogram (check 6): it needs
the two lengths alone". `samples.json` cases 116 and 117 pass unchanged.

### F-146 · ambiguity · telemetry.v1 0.1 §2.11 and §5: `incomparable`, a no-rate reason no pair reaches

> §2.11: "There is no rate across a reset, a decrease or an incomparable
> pair, and none of a histogram at this version"

> §5, the fifth question: "None, with why: `reset`, `decreased`,
> `incomparable`, `elapsed`. **Not asked:** not a counter (`kind`)."

> `continuity.json`: "`kind` (the pair is not of two counters), `reset`,
> `decreased`, `incomparable`, or `elapsed`"; case 39, a counter then a
> gauge, `unobservable`, with `no_rate` `kind`

By §2.10 point 2, a pair is incomparable when its kinds differ, or two
histograms' bounds or `counts` lengths differ. Two counters are therefore
never incomparable, and an incomparable pair always holds a sample that is
not a counter. That pair is both incomparable (§2.11's list) and "not a
counter" (§5's `kind`). Two readings follow:
- **The pair is incomparable:** §2.11's sentence, so `incomparable`.
- **The pair is not of two counters:** the fixture, so `kind`.

The fixture takes `kind`. That leaves `incomparable` in a closed list of
reasons that no pair can produce. A reader that orders §2.11's reasons
first fails case 39.

**zk2py:** `kind` first, as the fixture says. `incomparable` stays in the
code for a pair of two counters, which cannot be formed.

**Wording:** §2.11 and §5, drop `incomparable` from the rate's reasons, or
"`kind`: either sample is not a counter, an incomparable pair included".

**Status at telemetry.v1 0.2: resolved by 0.2, as the fixture said.**
§5's no-rate list is `reset`, `decreased`, `elapsed`, and "not of two
counters (`kind`), an incomparable pair included". zk2py dropped its
unreachable `incomparable` branch: such a pair has no rate for `kind`.

### F-147 · gap · telemetry.v1 0.1 §2.6 check 2, with scenarios.md §7 step 3: an Epoch attachment that does not decode, and "an empty string"

> §2.6 check 2: "**`epoch_invalid`:** an in-band epoch present (the
> Point's `epoch`, or the Epoch attachment, §2.4) that is not a string of
> 1 to 64 characters"

> §2.6: "A sample the core could not decode (bytes that are not its JSON or
> its CBOR) is unobservable here (`undecodable`)"

> scenarios.md §7 step 3: "*Raw:* `bytes` at 60, with an empty string as
> its attachment." Expected 3: "R reads step 3 as `does_not`,
> `epoch_invalid`"

`samples.json` gives each attachment already decoded ("the decoded
attachment"), so it has no attachment whose bytes do not decode. On the
wire:
- **The JSON text `""`.** The attachment's type is
  `json:telemetry.v1.Epoch`, so its bytes are JSON text, and §2.14's
  18 bytes count the quotes. An empty string is then two bytes, which
  decode to `""`: `epoch_invalid`.
- **Zero bytes.** "An empty string as its attachment" also reads as zero
  bytes. Measured, zenoh-python delivers a zero-byte attachment as present
  and empty (`b''`), which is no JSON value.

Is an attachment that is not JSON (zero bytes, or `x`) `undecodable`, so
that the sample is unobservable and its value goes unread? Or is it an
epoch present and not a string, `epoch_invalid`? §2.6's `undecodable`
names the payload ("its JSON or its CBOR"), not the attachment.

**zk2py:** `epoch_invalid`: the attachment is present and reads as no
string. §7 runs all three: the JSON `""` (the scenario's step 3), zero
bytes, and `x`. Each reads `epoch_invalid` and pairs with nothing.

**Wording:** §2.6 check 2, "an Epoch attachment whose bytes do not decode
as its encoding is an epoch present and not a string". scenarios.md §7
step 3, "the JSON string `""` (two bytes) as its attachment".

**Status at telemetry.v1 0.2: resolved by 0.2, as zk2py read it.** §2.6
check 2: "An Epoch attachment whose bytes do not decode as its encoding is
an epoch present and not a string, zero bytes included; the sample is
still read", and `undecodable` "names the payload alone". scenarios.md §7
step 3 now says "the JSON string `""` (two bytes)". `samples.json`'s four
`attachment_text` cases pass. Whether the attachment's JSON is strict is
left: F-151.

### F-148 · gap · telemetry.v1 0.1 §2.10 "in the order the reader received them", with core §4.2: a state member's pairs

> §2.10: "A **pair** is two conforming samples (§2.6) of one member, in
> the order the reader received them."

> core §4.2: "A late joiner MUST merge the GET's replies and the
> subscription's samples per key by the owner's stamp (S1, S2), whichever
> path brought each one, and keep the newer."

A reader of a state metric that seeds as core §4.2 says receives samples
by two paths, so the order it receives them in is not the stamp order:
- a GET's reply of 10 (stamp 1) can arrive after a live put of 12
  (stamp 2);
- paired in receipt order, 12 → 10 is a decrease;
- with the instance alone throughout, that is `decreased`, established:
  the finding, for a counter that never went down;
- through the merge, the reply is not kept, and nothing pairs.

`scenarios.md`'s members are all streams. `continuity.json`'s two state
cases (26 and 27) are in stamp order. So neither decides.

**zk2py:** for state, a pair reads the samples the merge keeps, in the
order it keeps them (`telemetry.kept_reads`, over `zk2py.seed.kept`). Two
`readings` checks show both answers.

**Wording:** §2.10, "… in the order the reader received them; for state,
in the order a late joiner's merge keeps them (core §4.2): a sample the
merge does not keep is no sample of a pair". A `scenarios.md` section
should seed a state counter.

**Status at telemetry.v1 0.2: resolved by 0.2, as zk2py read it.**
§2.10: "For state, it is in the order a late joiner's merge keeps them
(core §4.2) … a sample the merge does not keep … is no sample of any
pair". scenarios.md §9 runs it, and zk2py's run passes it against both
owners: 12 → 15 through the merge, 12 → 10 `decreased` in receipt order.

### F-149 · ambiguity · telemetry.v1 0.1 §2.6 check 1 and check 2, with core §5.2: a Point's optional member written `null`

> §2.6 check 1: "`labels` present and not an object of strings; `unit`
> present and not a string"

> §2.6 check 2: "an in-band epoch present (the Point's `epoch` …) that is
> not a string"

> §2.4: "`null` is no value of any kind, and absent is not zero (§2.12)"

> core §5.2: "A `cause` or `detail` that is `null` is absent."

A Point with `"epoch": null`, `"unit": null` or `"labels": null` has two
readings:
- **Present:** in JSON's data model, the member is there. `null` is not a
  string, so `epoch` gives `epoch_invalid`, and `unit` or `labels` gives
  `not_a_point`. §2.4's "`null` is no value" leans this way.
- **Absent:** the core reads its own envelope's optional members so
  (§5.2). So does a typed reader whose optional field defaults on
  `null`. The Point then conforms, with no in-band epoch and no unit.

The second reading changes a pair's evidence: a sample whose epoch is
`null` lends no in-band epoch, so its pair falls to presence. No
`samples.json` case writes an optional member as `null`.

**zk2py:** present, by the text's literal reading. `null` is not a
string, so `"epoch": null` is `epoch_invalid`, and a `null` `unit` or
`labels` is `not_a_point`.

**Wording:** §2.6 check 1, "a member written `null` is present: `null`
is not a string", and `samples.json` cases for the three members.

**Status at telemetry.v1 0.2: resolved by 0.2, as zk2py read it.** §2.6
"A member written `null` is present", and core §5.2's rule "is the
envelope's own". `samples.json` cases 122 to 124 pass.

### Read and not filed

These were read for and found decided:
- **Integral floats.** One rule, JSON Schema's `integer`, covers every
  integer the Point holds: a counter, a timestamp, a byte of `binary`, a
  count. `-0.0` is the counter 0. `1e300` is an integer out of range. The
  double 2^64 is out of range, and 2^64−2048 is not.
- **±0 bounds.** `-0.0` equals `0` as a double, so `[-0.0, 0]` is not
  ascending, and `[0, 1.0]` matches a declared `[-0.0, 1]`. JCS writes
  `-0.0` as `0` in a bundle, and the text compares by value, so a bundle
  reader and a source reader agree.
- **The rate's bits.** Python's `float(int)` is the nearest double, ties
  to even, and its division is IEEE's. Every fixture rate matches exactly,
  2^64−1 over one second included.
- **The nullable carrier.** Both spellings, and a nullable form with no
  reading, carry nothing. §2.4's table already matches none of them, so
  no special case is needed.
- **Events and operations.** `ignored` when they write a key, whatever its
  value, else `not_this_profiles`. That holds even for an event typed by
  the Point (case 79), by §0's "Metric resource", against §2.1's
  shorter "a Point-typed resource … is a metric resource".
- **The profile's other types as carriers.** A bare `telemetry.v1.Epoch`
  carries `text` by §2.4's table, and a bare `telemetry.v1.Value` carries
  nothing. No fixture covers either.
- **A freshness re-put of a state Point** carries the same `timestamp`,
  so its pair with the original has no rate (`elapsed`).

## At telemetry.v1 0.2 (#609)

0.2 answers F-142 to F-149 one by one, each answer stated in the text and
pinned by new fixture cases where a fixture can pin one. `samples.json`
gains three input forms, the bytes as they came (`json_text`, `cbor_hex`,
`attachment_text`), and 24 cases (125); `continuity.json` gains
`stamp_ntp64` and two cases (45). `scenarios.md` gains §9, a state counter
seeded through a late joiner's merge, and `meter.v1` gains `total`. The
standard schema, its ids, `declarations.json` and `schema.json` are
unchanged, and the core stays at 0.35.

Each answer, read against its finding (the status lines above):

| Finding | 0.2 | Against zk2py's reading? |
|---|---|---|
| F-142, a stamp's nanoseconds | uhlc's `as_nanos`, the fraction rounded up, each stamp alone | no |
| F-143, a JSON sample | strict JSON, every number finite | partly: `1e400` and 10^400 are now `undecodable` |
| F-144, CBOR bignums and tags | a bignum is its integer; any other tag its content | yes: a bignum was a byte string |
| F-145, `counts_length` | checked whenever the value is a Histogram | no |
| F-146, `incomparable` | dropped from the rate's reasons: `kind` | no (the fixture's reading) |
| F-147, an attachment that does not decode | `epoch_invalid`, the sample still read | no |
| F-148, a state member's pairs | in the merge's order (core §4.2) | no |
| F-149, a member written `null` | present | no |

**zk2py follows 0.2.**
- `telemetry.decode_json` refuses a number past the double's range, a
  float or an integer's text.
- `telemetry.decode_cbor` reads bignums as integers, through
  `zk2py.cbor.loads(…, bignums=True)`. The error envelope keeps core
  §5.2's reading.
- `telemetry.ntp64_ns` is §2.11's conversion.
- `continuity` gives `kind` for an incomparable pair.

Of the 24 new `samples.json` cases, round 45's decoders read seven
differently: `1e400`, 10^400, and the five bignums. Those are exactly the
answers that went against zk2py's reading.

**`just py-conformance`: 2199 of 2199.** At the start of the round, round
45's runner passed the 21 families before `telemetry`, then stopped there
with exit 2: it read `value` from a `json_text` case (a `KeyError`).

| Family | Before | After |
|---|---|---|
| telemetry | crashed (243 of 243 at 0.1) | 269 of 269 |
| readings | 172 of 172 | 179 of 179: four of F-150's and F-151's readings, three of a sample nested past the reader's depth (review of PR #826) |
| the 20 others | 1751 of 1751 | 1751 of 1751 |

**`just py-telemetry`, §1 to §9.** Every check passes, judged by zk2py's
reader, against both owners:

| § | zk2py's owner | The Rust owner example (`owner-tb`) |
|---|---|---|
| 1 A counter across a restart (+ the owner's fingerprint, a new session's stamps) | 12 of 12 | 12 of 12 |
| 2 A counter across `new_epoch` in one process | 6 of 6 | 6 of 6 |
| 3 A decrease within one epoch | 6 of 6 | 6 of 6 |
| 4 A Point whose tag disagrees | 5 of 5 | 5 of 5 |
| 5 Histograms, other bounds and a decrease | 5 of 5 | 5 of 5 |
| 6 Absence is not zero | 3 of 3 | 3 of 3 |
| 7 An in-band epoch on a bare counter (+ step 3's refusal, review of PR #826) | 6 of 6 | 5 of 5 |
| 8 A counter that crosses a face (R0, and R0F across a real face) | 8 of 8 | 8 of 8 |
| 9 A state counter seeded | 5 of 5 | 5 of 5 |
| §2.11's conversion against zenoh-python | 1 of 1 | 1 of 1 |
| **Total** | **57 of 57** | **56 of 56** |

What the black box showed:
- **Its `--help` was enough to drive it.** Nothing had to be found by
  trying. That covered its flags, its commands and its answers, the
  refusal's `<why>` included.
- **It embeds `meter.v1`,** and needs no contract file. Its descriptor's
  fingerprint is the one zk2py computes for the contract transcribed from
  `scenarios.md` (`sha256:cd33b1f7…`).
- **Its checked writer refused every value a scenario writes raw,** with
  the tokens zk2py's writer gives: `decreased` (§3, §5 step 3),
  `tag_disagrees`, `unit_disagrees`, `wrong_type` (§4) and
  `bounds_differ` (§5). §7's `""` attachment was refused as
  `epoch_invalid`. The same value then went out by `telemetry raw`.
- **What the driver supplies.** The writer fills the epoch where a put
  leaves it out, and refuses a Point with no `timestamp` (`not_a_point`),
  so the driver writes each timestamp from the host's clock. That matches
  its help, which says the JSON "is sent as written".
- **The one check it cannot run** is §7's undecodable attachments, zero
  bytes and `x`. Its `raw` takes an attachment's JSON alone ("the
  attachment is not one JSON text"). That check stays zk2py's.

### F-150 · ambiguity · telemetry.v1 0.2 §2.6 "Strict JSON": "past the double's finite range", at the edge

> §2.6 (0.2): "every number finite. A JSON sample that is not strict JSON
> is `undecodable`: `1e400` is, and so is an integer's text past the
> double's finite range (10^400 written out). An integer within that range
> is read exactly, however long its text."

The largest double is 2^1024 − 2^971. IEEE 754's round-to-nearest takes
every value below the halfway point to the next power, 2^1024 − 2^970, to
that largest double, and only values at or past it to +Infinity. That
leaves a band where two readings disagree:
- **The literal reading.** An integer's text one past the largest double
  is "past the double's finite range", so `undecodable`.
- **The rounding reading.** Its nearest double is the largest one, which
  is finite, so it reads and a gauge conforms.

Floats have the same band. The text `1.7976931348623158e308` is above the
largest double, and rounds to it.

`samples.json`'s cases (`1e400`, 10^400, 10^29) are far from the edge, so
none decides. Probed through its command line, the owner example takes
both edge values, as zk2py does. It refuses the halfway point's text
before sending it ("number out of range").

**zk2py:** the rounding reading. Python's `float()` of the text or of the
integer is finite (`telemetry._finite`), so both edge values read, as the
example's writer read them.

**Wording that would remove the guess:** §2.6, "every number finite: its
nearest double (IEEE 754, round to nearest) is not an infinity", and a
`json_text` case one past the largest double.

**Status at telemetry.v1 0.3: resolved by 0.3, as zk2py read it.** §2.6
"Finite" is by rounding: "a number's text, an integer's included, is
finite when its exact value, rounded to the nearest double … is not an
infinity", the edge the tie at 2^1024 − 2^970, which rounds to even,
2^1024. `samples.json` cases 125 to 130 pin both edges, as floats and as
integers. They pass unchanged, and round 46's reader passed them too.

### F-151 · gap · telemetry.v1 0.2 §2.6 check 2, with "Strict JSON": whether an Epoch attachment's JSON is strict

> §2.6 (0.2): "**Strict JSON.** A JSON sample is read as `alarms.v1` §2.4
> reads a payload … every string with a UTF-8 form (so no lone surrogate
> escape)"

> §2.6 check 2 (0.2): "An Epoch attachment whose bytes do not decode as
> its encoding is an epoch present and not a string"

> `samples.json`: "`attachment_text`, the attachment's bytes as UTF-8
> text, read as JSON"

"Strict JSON" names a sample's payload. Check 2 says only that an
attachment decodes "as its encoding". So it is open whether that decoding
is strict. Two readings give two answers for the attachment
`"\ud800"`, a lone surrogate escape:
- **Strict:** the attachment does not decode, so `epoch_invalid`.
- **RFC 8259 alone:** it is a string of one code point, so a valid epoch,
  and the sample conforms. Its epoch has no UTF-8 form, though, so two
  readers cannot even compare it for equality byte for byte.

No `attachment_text` case has one. The owner example cannot send one: its
command parser refuses the escape ("the attachment is not one JSON
text").

**zk2py:** strict, as for the payload. `decode_json` refuses the escape,
so the attachment reads `epoch_invalid`.

**Wording:** §2.6 check 2, "… do not decode as its encoding, read as
this section reads a sample (strict JSON, or CBOR)", and an
`attachment_text` case with a lone surrogate.

**Status at telemetry.v1 0.3: resolved by 0.3, as zk2py read it.** §2.6
check 2: "An Epoch attachment is read as this section reads a sample of its
encoding: strict JSON, or CBOR, depth included … and so is the JSON text
`"\ud800"`". zk2py now decodes an attachment by its `attachment_encoding`
(`telemetry.decode_attachment`). `samples.json` case 136 passes unchanged.

## At telemetry.v1 0.3 (#609)

0.3 answers F-150 and F-151 as zk2py read them. It adds three rules:
- **a data item's depth** (§2.6): at most 256 arrays and maps, a tag
  adding none;
- **presence that stands on a complete read** (§2.9), with a loss between
  two arrivals establishing neither the same epoch nor a different one;
- **a SHOULD for a tool over many pairs** (§5): over pairs whose every
  epoch was assumed, it says it could not tell.

`samples.json` gains twelve cases (137). Everything else is unchanged:
the schema, `declarations.json`, `continuity.json` and `schema.json`,
and the core, at 0.35.

**zk2py follows 0.3.**
- **Depth:** `telemetry.depth` counts arrays and maps without recursion,
  and a sample past `MAX_DEPTH` (256) is `undecodable`, in JSON and CBOR.
  `zk2py.cbor` reads a chain of tags without recursion, so a tag adds no
  depth.
- **The attachment:** an Epoch attachment is decoded by its own
  `attachment_encoding` (`decode_attachment`).
- **Presence:** R's view now stands on a complete liveliness read after
  its subscription (`Reader.seed_presence`, core §8.1), and gives no
  presence to an arrival before it.
- **§5:** `telemetry.verdict` answers over a window: a finding, no, or
  could not tell.

Of the twelve new cases, round 46's reader read two otherwise: arrays
nested 257 deep, in JSON and in CBOR. It decoded them up to Python's
recursion limit and read `wrong_type`. That is the new rule, and the
cases on F-150 and F-151 passed unchanged.

**`just py-conformance`: 2226 of 2226.**

| Family | Before | After |
|---|---|---|
| telemetry | 279 of 281 (the two depth cases) | 281 of 281 |
| readings | 179 of 179 | 194 of 194: seven of `verdict`, F-152's reading among them; five of tag chains and three of §2.9's seed folded (review of PR #828) |
| the 20 others | 1751 of 1751 | 1751 of 1751 |

**`just py-telemetry`, three runs, each exiting 0.**

| § | zk2py's owner | The Rust owner (`owner-tb`) | zk2py's owner, `zenctl telemetry` judging too |
|---|---|---|---|
| 1 | 12 of 12 | 12 of 12 | 18 of 18 |
| 2 | 6 of 6 | 6 of 6 | 12 of 12 |
| 3 | 6 of 6 | 6 of 6 | 12 of 12 |
| 4 | 5 of 5 | 5 of 5 | 11 of 11 |
| 5 | 5 of 5 | 5 of 5 | 10 of 10 |
| 6 | 3 of 3 | 3 of 3 | 10 of 10 |
| 7 | 6 of 6 | 5 of 5 | 11 of 11 |
| 8 | 8 of 8 | 8 of 8 | 14 of 14 |
| 9 | 5 of 5 | 5 of 5 | 10 of 10 |
| §5 over many pairs, four windows built for the tool | | | 17 of 17, and F-152's note |
| §2.11's conversion | 1 of 1 | 1 of 1 | 1 of 1 |
| **Total** | **57 of 57** | **56 of 56** | **126 of 126** |

**The tool against zk2py's owner.** `zenctl telemetry` was read from its
`--help` alone: `--for`, `-c`, `--across-face` with `--contracts`,
`--format json`, and exit 0, 1 or 2.
- **How each section runs it:** the tool starts after the owner is up,
  over a window covering the section. In §8 it stands across the real
  face, on R2's `unixsock-stream` endpoint, with `--across-face`. In §9 it
  starts after O's put of 10, so its own S4 GET seeds that value.
- **What is compared, member by member,** against zk2py's reader:
  - the kind and unit and where each came from;
  - the samples' counts (received, conforming, `does_not`,
    unobservable) and their problems;
  - each pair's continuity, epoch, evidence and time base;
  - each rate: exactly when timed by Points' timestamps or the owner's
    stamps, within 5 % when timed by receipt, each reader's own clock;
  - the exit code, against `verdict` over zk2py's reading;
  - and, in each window, that zk2py's reader read samples and that the
    tool read every one of them (review of PR #828).
- **The readers' counterparts:** R for §1 to §7, R0F across the face for
  §8, and in §9 a witness subscribed from the start, which reads the same
  three values in the owner's order.
- **They agree on every member of every section:**
  - §1 and §2's resets, by presence and in-band;
  - §3's two `decreased`, and its exit 1;
  - §4's three problems;
  - §5's `bounds_differ` and its decrease;
  - §6's −70.0, and no value for a member never written;
  - §7's `epoch_invalid` for the `""`, zero-byte and `x` attachments;
  - §8's assumed reset across the face, beside the Point's in-band one;
  - §9's merge: 10 → 12 → 15, by presence and the stamps.

  Every rate timed by timestamps or stamps is the same double.
- **The four windows built for §5's SHOULD:**
  - counters whose every pair was assumed, and histograms likewise: the
    tool exits 2 and zk2py says could not tell;
  - no sample at all: both exit 2;
  - a counter's one sample and no pair: the tool exits 0, where zk2py
    says could not tell (F-152).

### F-152 · ambiguity · telemetry.v1 0.3 §5 "Over many pairs": a counter read with no pair at all

> §5 (0.3): "A tool asked "did anything go backwards?" over pairs of
> counters or histograms whose every epoch was assumed SHOULD say it could
> not tell, never that nothing went backwards: no decrease could have been
> found"

The SHOULD names pairs whose every epoch was assumed. It does not name a
window that read a counter but formed no pair, one sample of it, say. Two
readings follow:
- **Vacuous.** Over no pair, "every epoch was assumed" holds, and so does
  the reason given: "no decrease could have been found". The tool should
  say it could not tell.
- **Non-vacuous.** The SHOULD speaks of pairs that exist. With none, the
  question had nothing to answer, and saying nothing went backwards is not
  a no without evidence, because no evidence was asked for.

`zenctl telemetry` reads the second way. Over one conforming sample of
`drops` it exits 0, and its judgement is `not_established`, "0 pair(s),
none decreased within an established epoch (0 established, 0 assumed)".
That is the "nothing went backwards" the SHOULD forbids when pairs exist.
zk2py reads the first way. Its reason ends the same way the text's does:
nothing could have been found. No fixture decides, since `continuity.json`
is per pair.

**zk2py:** could not tell (`telemetry.verdict`), recorded as a note beside
the tool's exit 0, never a failure.

**Wording that would remove the guess:** §5, "… whose every epoch was
assumed, or that formed no pair of a counter or histogram it read, SHOULD
say it could not tell", or the opposite, stated.

**Status at telemetry.v1 0.4: resolved by 0.4, as zk2py read it, and asked
of each member.** §5: "… or that formed no pair of a counter or histogram
it read, SHOULD say it could not tell", and "Each counter or histogram read
is asked … a pair established in-band of one says nothing of another".
`telemetry.verdict` now asks it of each member (`member_answer`).
`zenctl-te` exits 2 over a counter's one sample, as zk2py does. It exits 2
too beside another counter's established pairs, the new probe. §8 across
the face now exits 2, for `drops`, as 0.4 says.

### F-153 · gap · `zenctl telemetry --help` (#819, TC): "every counter's pair assumed", and histograms

> `zenctl telemetry --help`: "2 no verdict: no metric resource in scope,
> no sample in the window, every counter's pair assumed (presence
> unobservable throughout, no in-band epoch)"

> §5 (0.3): "over pairs of counters or histograms whose every epoch was
> assumed"

The help names counters, and the text names counters or histograms. Read
as written, a window of histograms whose every pair was assumed exits 0.
Probed, it exits 2, "every one of the 2 pair(s) of a counter or histogram
had its epoch assumed". The tool does what the text says, and its help
says less.

**zk2py:** read from the tool's behaviour, which matches the text. §5's
probes check both.

**Wording:** the help, "every counter's or histogram's pair assumed".

**Status at telemetry.v1 0.4: answered in the tool.** `zenctl-te`'s help
reads "every counter's or histogram's pair assumed", and the histogram
probe exits 2, as before.

### F-154 · gap · `zenctl telemetry`'s stderr: "listening" before it listens

> stderr, about 10 ms after the start: "telemetry: listening 6s to the
> metric members in scope and their owners' tokens (--for)"

> `--help`: "--for <SECS> The reading's window, seconds: the subscriptions
> listen this long"

The line comes before the tool has read presence, the descriptors and the
contracts, and so before its subscriptions are declared. Measured:
- a sample put within about 10 ms of the line was lost in two of two
  tries;
- one 50 ms later or more was kept, in four of four.

So a caller that starts its steps on the line loses the first sample of
the window, and the window's start is not observable. The tool's
judgement then misses that sample: in one probe, `drops` 10 followed by a
raw 7 read as one conforming sample of 7, so the decrease was never
paired and the run exited 0.

**zk2py:** starts its steps 1 s after the line (`TOOL_MARGIN_S`), and
checks that the tool read every sample zk2py's reader did. It did, in
every section.

**Wording:** print the line once the subscriptions are declared and
presence seeded, or say in `--help` how long the window takes to open.

**Status at telemetry.v1 0.4: answered in the tool.** `--for` "opens once
the subscriptions are declared and presence is seeded, when stderr says
"listening": a sample put from then on is inside it". zk2py dropped its
1 s margin, and its steps start the moment the line appears. A probe puts
a sample at once, in three tries, and the tool read it in all three. Every
section's "read every sample" check passes with no margin.

### F-155 · gap · `zenctl telemetry --format json`: a document the help does not describe, and pairs cut to the last 16

> `--help`: "--format … json (one document) or ndjson (one object per row)
> for scripts"

The help states the semantics, and not one field of the document. A
script reading it has to infer each of these from runs:
- **The judgement's `answer`.** It was `established` with exit 1,
  `not_established` with exit 0 and `unobservable` with exit 2. The last
  two stand beside "none decreased" and "could not have been found".
- **A pair's fields:** `base` (`timestamp`, `stamp`, `arrival`), `via`
  (absent for an assumed pair) and `presence` (`observed`, or `none`
  across a face).
- **A member's `pairs`,** which holds the last 16 pairs alone, while its
  `continuity.pairs` counts all of them. Measured: 25 samples gave 24
  pairs, and 16 were listed, from 8 → 9. Nothing in the row marks the cut.
  A script that counts the list, or reads its first pair as the window's
  first, is wrong without knowing it.

**zk2py:** compares the tool's pair count with its own, and the listed
pairs with the tail of its own. The sections have at most four pairs per
member, so none was cut here.

**Wording:** the help, or a pinned schema it names, stating the
document's fields. A row should say when its `pairs` is a tail, for
instance with `pairs_listed` beside `continuity.pairs`.

**Status at telemetry.v1 0.4: answered in the tool.** The help names the
document's fields and their tokens, and a member row's `continuity`
carries `pairs_listed`, "the latest 16, fewer than `pairs` when it is a
tail". A probe of 25 samples reads `pairs` 24, `pairs_listed` 16 and 16
listed, the last ending at the last sample. The comparator checks
`pairs_listed` against the list in every row, and each member's
`continuity.answer` against zk2py's `member_answer`.

### Read and not filed

- **§2.9's complete read.** "A subscription's history alone" is not a
  complete read, and core §8.1 says what one is. zk2py's R now makes one
  after subscribing, and every section's presence evidence held.
- **§2.9's loss between two arrivals.** zenoh-python's callback handlers
  lose nothing, so no run here has a loss to read. `continuity.json`'s
  presence input already says what a reader saw.
- **The depth's edges:** 256 and 257, in JSON and in CBOR, and a tag
  around 256. They are pinned by `samples.json` and agree with zk2py.

## At telemetry.v1 0.4 (#609)

0.4 answers F-152 the vacuous way, as zk2py read it, and asks §5's SHOULD
of each counter or histogram read. Its last bullet states the reference
tool's exit codes. The tool's own findings, F-153 to F-155, are answered in
`zenctl-te` (`zenctl 0.12.0 (0.14.0-768-ge1c64ab9)`). Nothing else changed:
the schema, every fixture and the core.

**zk2py follows 0.4.**
- **`telemetry.member_answer`:** one member's continuity answer, in the
  judgement's tokens.
- **`telemetry.verdict`:** asks it of each member. A counter or histogram
  that formed no pair, or only assumed ones, could not tell, whatever
  another formed, and a finding stands beside it.
- **The comparator:** checks each member row's `continuity.answer`
  against `member_answer`, and its `pairs_listed` against the list.
- **The margin:** `TOOL_MARGIN_S` is dropped. The steps start on the
  "listening" line.

**`just py-conformance`: 2231 of 2231.** The readings family gains five
checks of `verdict`: two members, one established and one with no pair or
only assumed ones; a finding beside one that could not tell; a gauge beside
two established counters; and F-157's reading. `telemetry` is unchanged at
281.

**`just py-telemetry`, three runs, each exiting 0:** 57 of 57 against
zk2py's owner, 56 of 56 against `owner-tb`, and 140 of 140 with `zenctl-te`
judging beside zk2py's reader.
- **The tool agrees on every member of every section,** its exit code and
  each member's `continuity.answer` included.
- **§8 now exits 2.** `drops` across the face forms only assumed pairs,
  and `frames`' in-band epoch answers nothing of it, which is what 0.4
  says. zk2py's verdict says the same.
- **§5's probes:** seven windows, each in agreement:
  - counters whose every pair was assumed, and histograms likewise;
  - a counter's one sample;
  - one counter established beside another with one sample;
  - no sample at all;
  - a gauge beside an undecodable sample of it;
  - and the F-154 and F-155 checks above.

**The whole-text pass.** I read `v1.md` and `scenarios.md` through once
more, as a reader new to both, against everything zk2py implements. Three
places are left to a guess.

### F-156 · contradiction · telemetry.v1 0.4 §2.6 "How a value is read": two definitions of a CBOR integer

> §2.6, "CBOR" (0.2): "An integer is CBOR's major type 0 or 1, or a
> bignum, tag 2 or 3 around a byte string of any length, definite or not,
> read as the integer it denotes"

> §2.6, a few bullets on: "**An integer is read exactly:** JSON text with
> no fraction and no exponent, CBOR's major types 0 and 1."

The second bullet still defines a CBOR integer as major types 0 and 1
alone. That was 0.1's wording, which F-144 overturned. Read alone, the
second bullet makes a bignum no integer: `wrong_type` for 2^64 in a
counter, which was round 45's reading. Read with the first, the bignum is
the integer, `out_of_range`. `samples.json` cases 109 to 113 decide for
the first, but the text says both.

**zk2py:** the first, as the fixture says.

**Wording that would remove the guess:** the second bullet, "… CBOR's
major types 0 and 1, and its bignums (above)".

**Status at telemetry.v1 0.5: resolved by 0.5, as the fixture said.** The
bullet now reads "… CBOR's major types 0 and 1, and CBOR's bignums (tags 2
and 3, the CBOR reading above; 0.5)". No case changed.

### F-157 · ambiguity · telemetry.v1 0.4 §5, the reference tool's exit codes: is an undecodable sample a question answered?

> §5 (0.4): "The reference tool, `zenctl telemetry`, exits 1 on a finding,
> 2, the no verdict, when any counter or histogram it read could not tell,
> and 0 only when every question it asked was answered."

> §5: "'unobservable' (it was put, and the observation could not answer
> it)"; the table, "Does this sample conform? … **Unobservable:**
> `declaration_invalid`, `undecodable`"

> §2.6: "A sample the core could not decode … is unobservable here
> (`undecodable`): the core's verdict stands, and this profile reads
> nothing of it."

Read with §5's own definition, an undecodable sample is a question put
that the observation could not answer. So a window that holds one is not
"every question it asked … answered", and the tool should exit 2. Read
with §2.6, the profile "reads nothing of it", so its question is not this
profile's to have asked, and the window can exit 0.

Measured with `zenctl-te`:
- a gauge's conforming sample beside an undecodable one exits 0;
- a counter's two established samples beside an undecodable one of it
  exits 0;
- a counter whose only sample is undecodable exits 2, since "no pair
  formed".

zk2py reads it as the tool does, so the two agree. The bullet as written
supports the other answer. A declaration that is invalid (`unobservable`,
`declaration_invalid`) raises the same question; no section has one.

**zk2py:** an undecodable sample keeps a window from no answer only through
its member's continuity (`verdict`). A readings check and a tool probe pin
it.

**Wording:** §5, "… and 0 only when every question it asked was answered,
a sample's conformance answered unobservable (undecodable, or an invalid
declaration) aside", or "… 2 when any sample's conformance was
unobservable" if the other reading is meant.

**Status at telemetry.v1 0.5: resolved by 0.5, against zk2py's reading
and the tool's.** §5 gains "Over many samples": "A tool asked "did every
sample conform?" over a window SHOULD say it could not tell when any sample
in it was undecodable, or any member it read had an invalid declaration".
"Every question of such a member is unanswered", its continuity included,
and "Only what was read counts".
- **zk2py:** `telemetry.verdict` now could not tell over either. A member
  read under an invalid declaration answers `unobservable` in
  `member_answer`.
- **`zenctl-tf`** exits 2 over both, and 0 when the invalid resource is in
  scope but unheard.
- **Checked:** both agree on all three, through a second interface on the
  owner's service, `meterx.v1`, whose `stream/odd` is declared `bool`.

### F-158 · gap · telemetry.v1 0.4 scenarios.md "Conventions": R's presence on a subscription's history alone

> scenarios.md, Conventions: "**R** … holds a liveliness subscription to
> `zk2/lab/meter/@zk/instance/*`, with history, both declared before the
> owner starts … Its presence is what R saw: alone throughout when exactly
> one instance token was held at every instant …"

> v1.md §2.9 (0.3): "A reader's view of them stands on a complete read of
> the tokens (core §8.1, which a subscription's history alone is not) …
> a view that stands on no complete read … establish[es] nothing"

The conventions describe R exactly as 0.3 says a reader must not stand: on
a subscription with history, and no complete read. A runner built from
`scenarios.md` alone, as zk2py's was in round 45, would follow it. A
strict reader of §2.9 then establishes nothing by presence, and §1's,
§2's, §3's and §9's "established by presence" do not hold. `scenarios.md`
was updated at 0.2 and 0.3 for other things, and not for this.

**zk2py:** R makes a complete liveliness read after its subscription
(round 47's `Reader.seed_presence`), retried, and folded so that the
subscription's word wins. Every section's presence evidence holds.

**Wording:** the conventions' R, "… then makes a complete liveliness read
of them (core §8.1), on which its view of presence stands (§2.9, 0.3)".

**Status at telemetry.v1 0.5: resolved by 0.5, as zk2py runs it.** The
conventions' R "makes a complete liveliness read of the same tokens (core
§8.1), on which its view of presence stands", with "the subscription's
moves … folded after it, the subscription's word winning where the two
disagree". That is round 47's `Reader.seed_presence` and `fold_seed`.

### Read and not filed

- **Which members are asked of continuity.** The changelog says a member
  whose kind is unobservable is not asked: an undeclared Point that never
  conformed, for instance. `v1.md` says "each counter or histogram read",
  which does not hold of such a member, so it is read the same way.
  zk2py and the tool both read it so.
- **§5's per-pair answers** are unchanged by 0.4: an assumed pair stays a
  no, and its SHOULD is about a window.
- **Everything else** in §2.1 to §2.15, §3, §4 and the appendices read as
  zk2py implements it, and every scenario's expectations hold against all
  three runs.

## At telemetry.v1 0.5 (#609)

0.5 answers F-156 to F-158:
- **F-156:** the integer bullet names bignums.
- **F-157:** §5 gains "Over many samples". Against zk2py's reading, an
  undecodable sample, or a member read under an invalid declaration,
  leaves a window unable to tell.
- **F-158:** `scenarios.md`'s R makes a complete liveliness read.

The schema, every fixture and the core are unchanged. The tool is
`zenctl-tf`, `zenctl 0.12.0 (0.14.0-778-g12659a0c)`.

**zk2py follows 0.5.**
- **`telemetry.verdict`:** could not tell over any sample read
  unobservable.
- **`member_answer`:** a member read under an invalid declaration answers
  `unobservable`, its continuity included. An undeclared Point never
  decoded is still not asked (0.4).
- **The probes:** the owner's service now also implements
  `meterx.v1`, zk2py's own interface (`interop/telemetry/meterx.v1.toml`),
  whose `stream/odd` is declared `bool` (`kind_value`). It is served
  throughout §5's probes, so every probe but one checks that "Only what was
  read counts".

**`just py-conformance`: 2235 of 2235.** The readings family gains four
checks, and F-157's check now expects could not tell:
- a member under an invalid declaration beside a gauge;
- its continuity `unobservable`;
- a finding beside it;
- an undeclared Point never decoded, still not asked.

`telemetry` is unchanged at 281.

**`just py-telemetry`, three runs, each exiting 0:**
- **zk2py's owner:** 57 of 57;
- **`owner-tb`:** 56 of 56;
- **`zenctl-tf` judging beside zk2py's reader:** 151 of 151, with three
  windows built for 0.5. The tool and zk2py agree on every member, answer
  and exit in each:
  - a gauge's sample beside an undecodable one: 2;
  - a member heard under an invalid declaration beside a valid gauge: 2,
    the member's `continuity.answer` `unobservable`, its samples
    `declaration_invalid`;
  - the invalid resource in scope and unheard: 0.

**The text is closed.** I read 0.5's changes with full care: §2.6's
integer bullet, §5's "Over many samples" and its reference-tool bullet,
and the scenarios' R. Round 48's whole-text pass covered the rest. Nothing
in `v1.md` or `scenarios.md` is left to a guess. No finding is open
against the text.

### F-159 · gap · `zenctl telemetry --across-face` (`zenctl-tf`): "listening" is still no signal across a face

> `--help`, `--for`: "It opens once the subscriptions are declared and
> presence is seeded, when stderr says "listening": a sample put from then
> on is inside it."

F-154's answer holds when the tool reads presence:
- a sample put the moment the line appears was read 5 of 5 times, and
  3 of 3 in the probe.

With `--across-face` there is no presence to seed, and it does not hold.
A sample put the moment the line appears was lost:
- 2 of 15 tries with `zenctl-tf`;
- 1 of 5 with `zenctl-te`;
- 1 of 5 in the run's own measurement.

None of 15 was lost when put 50 ms after the line. Round 48's across-face
probes passed by chance.

A caller that takes the help at its word loses the first sample of a
window across a face. It then reads one pair fewer, and its `drops` can
show `same_epoch` where a reset was put first.

**zk2py:** its `--across-face` runs alone start their steps
`ACROSS_FACE_MARGIN_S` (0.2 s) after the line. Each run measures the
margin-free case five times and notes it. Every check passes.

**Wording, or a fix:** in the tool, print "listening" across a face only
once the subscriptions reach the router, or state the case in `--help`.

**Status after TG: resolved in the tool (`zenctl-tg`).** The "listening"
line now comes after a round trip that reaches the owner's own session:
- with no face, each owner instance's descriptor GET;
- across a face, S4's GET of a state metric resource in scope;
- with neither, `--settle`, 0.25 s by default, "a margin and never proof".

zk2py dropped `ACROSS_FACE_MARGIN_S` and asserts each opening, 20 tries
of 20, with a sample put the moment the line appears. None was lost, and
each line's wording is the help's (`tool_opening`). The line names which
opening it stood on only when it fell back on `--settle`: F-160.

## At telemetry.v1 0.5, the tool after TG (#609)

Only the tool changed: `zenctl-tg`, `zenctl 0.12.0
(0.14.0-793-gb7aad71d)`. Its "listening" line now comes after a round
trip that reaches the owner's own session. The text and every fixture are
0.5's, unchanged.

**F-159: resolved.** `tool_opening` asserts each of the three openings,
20 tries each, with a sample put the moment the line appears. There is no
margin any more.
- **Across the real face, `total` in scope and holding a value:** 20 of
  20 read. The line names no `--settle`, so the opening stood on S4's GET.
- **Across the real face, streams alone in scope:** 20 of 20 read. The
  line says "this line comes after a 0.25s margin (--settle), never
  proof".
- **With no face:** 20 of 20 read. The line names no owner that did not
  answer.

One more case, met while reading the help: across a face, with a state
metric resource in scope that holds no value, the owner's S4 GET answers
with no reply. Nothing then shows it reached an owner, and the line says
so: "the owner answered none of 1 GET(s) … this line comes after a 0.25s
margin (--settle), never proof". That is right by core S2, where a GET of
a key with no value answers nothing.

**`just py-conformance`: 2235 of 2235**, unchanged.

**`just py-telemetry`, three runs, each exiting 0:**
- **zk2py's owner:** 57 of 57;
- **`owner-tb`:** 56 of 56;
- **`zenctl-tg` judging beside zk2py's reader:** 157 of 157, the six
  checks of `tool_opening` added. The tool agrees with zk2py on every
  member, answer and exit, as before.

### F-160 · gap · `zenctl-tg`'s "listening" line: a proven opening is named by silence

> `--help`, `--for`: "It opens once … each owner instance in scope answered
> its descriptor GET, when stderr says "listening" … (an owner that did not
> answer, the line names). Across a face … it opens once S4's GET of a
> state metric resource in scope was answered after the subscriptions;
> with none in scope, after --settle, a margin and never proof, as the
> line says."

The line names its opening only when it fell back on `--settle`, and
names any owner that did not answer. When a round trip proved the
opening, the line says nothing of it:
- **With no face:** "telemetry: listening 0.6s to the metric members in
  scope and their owners' tokens (--for)", with no word of the descriptor
  GETs.
- **Across a face, on S4's GET:** the line is the face's sentence alone,
  with no word of which GET was answered.

A caller can tell which opening the tool stood on only by the absence of
`--settle`, read with the help. Nothing is lost by it, since the tries
above found every sample, but the line does not say which one it stood on.

**zk2py:** reads the opening from the words the help defines: `--settle`
and "never proof" for the margin, and their absence for a proof.

**Wording, in the tool:** name the round trip, for instance "after N
owner(s)' descriptor GETs" or "after S4's GET of zk2/…/state/total".

**Status after TH: resolved in the tool (`zenctl-th`).** When a round
trip proved the window, the line now says so:
- with no face, "; each of the N owner instance(s) in scope answered its
  descriptor GET after the subscriptions, so they hold them";
- across a face, "; across the face the owner answered A of N S4 GET(s) of
  a state metric resource in scope after the subscriptions, so it holds
  them".

`tool_opening` now checks each opening's words positively, anchored on
"telemetry: listening", 20 lines of 20 each. Round 50's words for a proven
opening no longer pass.

**None open against the text.**

## At telemetry.v1 0.5, the tool after TH (#609)

Only the tool changed: `zenctl-th`, `zenctl 0.12.0
(0.14.0-794-g905b3684-dirty)`, with TH's code. Its "listening" line now
says what proved the window. The text and every fixture are 0.5's,
unchanged.

**F-160: resolved.** `tool_opening` (`--only opening`, its own selector)
checks the three openings, 20 tries each:
- **Each sample** put the moment the line appears is read, 20 of 20.
- **Each line** starts "telemetry: listening" and says the opening it
  stood on, positively. Across the face with `total` in scope, "the owner
  answered 1 of 1 S4 GET(s) of a state metric resource in scope after the
  subscriptions, so it holds them". Across it with streams alone, "a 0.25s
  margin (--settle), never proof". With no face, "each of the 1 owner
  instance(s) in scope answered its descriptor GET after the
  subscriptions, so they hold them". Every GET named is answered.
- **`total`'s value** is checked held, by an S4 GET, before the first
  case.
- **Each exit** is zk2py's verdict over the same sample: a counter's one
  sample could not tell, 2. zk2py's reader is first checked to have read
  the sample, so the comparison cannot pass over an empty view.

**`just py-conformance`: 2235 of 2235.**

**`just py-telemetry`, three runs, each exiting 0:**
- zk2py's owner, 57 of 57;
- `owner-tb`, 56 of 56;
- `zenctl-th` judging beside zk2py's reader, 164 of 164.

**None open.**

**The close of telemetry.v1's rounds, 45 to 51.** zk2py read
telemetry.v1 from its first text, 0.1, to 0.5, and judged the Rust
runtime's owner and then `zenctl telemetry` beside its own reader. It
filed nineteen findings, F-142 to F-160, and every one is resolved:
- **Fourteen against the text,** F-142 to F-152 and F-156 to F-158, were
  each answered by a later text. F-143, F-144 and F-157 went against
  zk2py's first reading.
- **Five against the tool,** F-153 to F-155, F-159 and F-160, were
  answered in the tool.

The implementations now agree on the following, measured against zk2py's
owner and `owner-tb`, and with the tool judging every section of
`scenarios.md`:
- **How a sample's bytes are read:** strict JSON finite by rounding, CBOR
  bignums as integers, a depth of 256, an attachment by its own encoding,
  `null` present.
- **A member's kind and unit**, and a sample's conformance and its
  reasons.
- **A pair's epoch:** in-band, then presence standing on a complete read,
  else assumed. Its continuity, and a state member's pairs through the
  late joiner's merge.
- **The rate,** bit for bit, a stamp's nanoseconds as uhlc reads them.
- **A window's answer, member by member:** a finding stands; could not
  tell over a counter or histogram with no established pair, an
  undecodable sample, or a member under an invalid declaration; and no
  only when every question asked was answered.
- **When a tool's window opens,** and the words that say so.

## At desired.v1 0.1 (#609)

`desired.v1` 0.1 is the eighth profile (`spec/profiles/desired/`): a
desired document is state of its author, keyed by its target, which binds
its own slice (core R2), converges, and reports in a standard
`desired.v1.Report`. zk2py read it cold with core 0.36, which publishes
its vocabulary, `target_param` alone. `zk2py.desired` is written from the
text alone:
- the standard schema's id and `is_profile_type` (§3);
- the declaration (§2.2), the binding and the slice's key (§2.3);
- stamps (§2.4) and their order (§2.6);
- the target as a pure state machine (§2.6 to §2.10), equal documents
  (§2.7);
- the report's reader and its eight findings (§2.10);
- §2.17's conclusions.

**Core 0.36.** Appendix D's published table gains `desired` with
`target_param`, and the interim row, with its `target`, is gone. zk2py
moved the key to its published table, so `desired.target` draws W105, the
new `contracts/w105-desired-target`, which failed until then. The
examples' two desired contracts, `mission_plan.v1` and `zs.desired.v1`,
write `target_param` alone and still load with no finding. §10's and
§4.4's pointers and Appendix C's row change nothing zk2py reads. Nothing
was found in core 0.36 itself.

**`just py-conformance`: 2572 of 2572.** At the start of the round, 2236
passed and 2 failed: `layout`, since the profile's `conformance/` was
unread, and `contracts/w105-desired-target`.

| Family | Before | After |
|---|---|---|
| layout | 20 of 21 | 21 of 21 |
| contracts | 102 of 103 | 103 of 103 |
| desired | (none) | 291 of 291 |
| readings | 203 of 203 | 246 of 246 |
| the 19 others | 1911 of 1911 | 1911 of 1911 |

The `desired` family covers six files, 222 cases, every one passing the
first time it ran:
- `schema.json`: its 3 cases, and five checks of its header against
  zk2py's tables (the current id, the published ids, the three types with
  their first revision, each a `$defs` member of the file);
- `declarations.json`: 37;
- `bindings.json`: 21;
- `order.json`: 64, and each case's report read back by zk2py's own
  report reader with no finding (64 more checks): §2.10's findings are
  "each one the state machine never produces";
- `reports.json`: 66;
- `conclusions.json`: 31.

Of the 43 new `readings`, ten read Appendix B and §2.3 over the adopters'
real contracts and bindings, which the fixtures give already resolved:
- the examples' desired resources are the walkthrough's `plans/{vehicle}`
  and `zs.desired.v1`'s seven templates, none invalid;
- the walkthrough executor's binding (`acl/walkthrough.enrollment.toml`)
  gives `…/plans/vehicle-01`, as core `bindings.md` §2 says;
- `zs.desired.v1`'s seven read `[host, service]`, never stale;
- `zs.thresholds.v1`'s requirement, bound for the sysinfo sensor, gives
  §2.3's example key, and unbound is the kill switch;
- its `applied/{topic}` is `AppliedConfig`, and no example resource is
  typed `desired.v1.Report`, as Appendix B says;
- the scenarios' `tasking.v1` gives `D` for `lab/worker`.

Five read zk2py's own refusal of a target address that is not
`<system>/<service>`, each a plain chunk (`target_address`, outside §2.3's
list, checked alone). The other 28 read the open findings' readings
(F-162, F-163, F-166, F-167, F-168), §2.7's equality over a payload's
bytes and over values RFC 8785 refuses, §2.10's report puts, and a
report's bytes no fixture gives (nested 100,000 deep, read as `json` and
never a crash; 300 deep, read; a byte-order mark; a lone surrogate).

**Passing is not deriving.** Three fixture expectations decide what the
text leaves open or says otherwise (F-161, F-164, F-165). Two more
readings pass every `order.json` case whichever way they are taken
(F-162, F-163): zk2py substituted each alternative into its state machine
and ran the file again, and nothing failed.

**The scenarios** need an author and a target on a bus, which #840's DB
brings with the owner example's `--desired`. This round read them for
sufficiency only, and built no harness. F-175 to F-180 are what a second
implementation could not run from their text alone.

### F-161 · contradiction · desired.v1 0.1 §2.3 "A binding is invalid": `wrong_self` of a literal or an unbound parameter

> §2.3: "`wrong_self`: of two target parameters, the first not bound to
> `self.system`, or the second not to `self.service`."

> `bindings.json`, description: "wrong_self (of two target parameters, one
> bound to a self other than its own: the first is self.system's, the
> second self.service's)"

Read literally, the text makes every wrong binding of a two-parameter
template `wrong_self` too: a first parameter bound to a literal is "not
bound to `self.system`", and an unbound second is "not [bound] to
`self.service`". The fixture's last case binds `host` to `h-0000` and
leaves `service` unbound, and expects `[not_one_author, target_unbound,
not_self]`, with no `wrong_self`. The description's narrower words decide
it.

**zk2py:** the description's reading. `wrong_self` is a parameter bound
to the other `self`; a literal is `not_self`'s alone, and an unbound one
`target_unbound`'s.

**Wording that would remove the guess:** §2.3, "`wrong_self`: of two
target parameters, one bound to the other's `self`: the first to
`self.service`, or the second to `self.system`".

**Status at desired.v1 0.2:** resolved by 0.2, as zk2py read it. §2.3:
"`wrong_self` (0.2): of two target parameters, one bound to the other's
`self` … A literal is `not_self`'s alone, and an unbound parameter
`target_unbound`'s alone." Four `bindings.json` cases pin it; zk2py passed
them unchanged.

### F-162 · contradiction · desired.v1 0.1 §2.6 and §2.7 "One act at a time": which held version a sample is judged against while an act runs

> §2.6: "It judges each version it receives against that one, or against
> the one it holds waiting while an act runs (§2.7)"

> §2.7: "While an act runs, a target holds the latest version it admits,
> judged against the running act's and then against the held one, the
> later replacing the earlier"

> §2.7: "records it as acted on when the act starts: a later delivery of
> that version is the same version."

> `order.json`, description: "A version is admitted against the one held
> waiting if there is one, else the last acted on."

§2.6 and the fixture's description judge against the held version alone;
§2.7 judges against the running act's first. The two differ when the held
version has another id than the running one. An act runs on `100/a1`, and
`50/b2`, a new epoch, is held. `100/a1` arrives again, by a second seed or
an archive. Against the running act's it is the same version. Against the
held one alone it is another id, so it is admitted and held, replacing
`50/b2`, and is acted on a second time in one instance: §2.7's "once per
version" MUST is broken. The description's own `same_version` ("the
running or last act's, or the one held waiting") names the running act's
too. No `order.json` case has two ids during one act: zk2py ran the
held-alone reading against the file, and every case passed.

**zk2py:** §2.7's, against the running act's version, then the held one
(`TargetRole.receive`). Two readings pin it.

**Wording that would remove the guess:** §2.6, "against that one, and,
while an act runs, against the running act's and then the one it holds
waiting (§2.7)", the same in the fixture's description, and an
`order.json` case with two ids during one act.

**Status at desired.v1 0.2:** resolved by 0.2, as zk2py read it. §2.6 now
judges "against the running act's version and then against the one it holds
waiting (§2.7)", and `order.json`'s description says so; cases 64 to 66 pin
it, and zk2py passed them unchanged.

### F-163 · contradiction · desired.v1 0.1 §2.6 "Never an older version" against step 4

> §2.6 step 4: "A target accepts the first value under a new id, admits
> it, and orders by time within the new id from then on, whatever the two
> times say"

> §2.6: "**Never an older version.** Within one id, a target never acts on
> a version older than one it acted on, in one instance, by any path: the
> owner's reply, its subscription, an archive's last-known (S12: zero stale
> plans applied)."

> §2.6: "a value of the author's old epoch still in transit when its new
> epoch's first value has been acted on is acted on as another id"

Step 4 compares a version with the one held alone, and so does the
fixture's description ("else the last acted on"). The second bullet
forbids, per id, an older version than any one acted on in the instance,
which needs a memory of every id's latest time. They disagree on an old
id's older version after the new id's first. A target acts on `500/a1`,
its author starts a new epoch, and it acts on `100/b2`. Then an archive's
stale last-known `400/a1` arrives. Against `100/b2` it is another id, so
it is admitted and acted on. By the bullet it is older than `500/a1`
within `a1`, and MUST NOT be: a stale plan applied, which S12 measured
none of. The third bullet's in-transit value is later than its epoch's last acted
version, and passes both. No `order.json` case has an old id's older
version after a new id: zk2py ran the per-id reading against the file,
and every case passed.

**zk2py:** step 4's, the held version alone (`TargetRole._judge`). A
reading pins it, with the archive's stale `400/a1` acted on and reported
`last_known`.

**Wording that would remove the guess:** either the bullet narrowed, "in
one id, a target never acts on a version older than the one it holds", or
step 4 widened, "and never on a version older than one it acted on under
that id", with an `order.json` case either way.

**Status at desired.v1 0.2:** resolved by 0.2, against zk2py's reading.
Step 4: "**But never an older version of an id it remembers**", per id,
"for at least the 16 most recent ids", from whichever path. zk2py had
judged against the held version alone; `TargetRole.remembered` now keeps
exactly 16 ids, most recently acted on or adopted last. `order.json` cases
67 to 72 pin it (case 72 fails with 15).

### F-164 · gap · desired.v1 0.1 §2.4 and §2.10: a stamp's id of 0

> §2.4: "its NTP64 time in decimal, without leading zeros, then `/`, then
> its id in lowercase hexadecimal without leading zeros (core §4.1,
> Appendix B)"

> §2.10: "a stamp in the form of §2.4, its time at most 2^64−1 and its id
> at most 128 bits"

The time and the id have the same form, "without leading zeros", and the
text bounds each from above only. `reports.json` reads the time `0` (case
18, `0/1`) and refuses the id `0` (case 41, `100/0`, `stamp`). Its
description says it apart: the time is "0 or without a leading zero", the
id "without a leading zero". What makes the id `0` no stamp is that a
zenoh id is never 0, which neither the text nor core Appendix B states.

**zk2py:** the fixture's: the id is `[1-9a-f][0-9a-f]*`, at most 32
digits.

**Wording that would remove the guess:** §2.4, "its id, never 0, in
lowercase hexadecimal without leading zeros", and core Appendix B, "an id
is never 0".

**Status at desired.v1 0.2:** resolved by 0.2. §2.4: "The id is never 0,
since a zenoh id is never zero". `reports.json` case 66 (`0/0`) passed
unchanged.

### F-165 · ambiguity · desired.v1 0.1 §2.10 "A target's report MUST be consistent": an absent member

> §2.10: "`in_force`: an `in_force` with a `source` other than `desired`,
> or with `failed`; or, with `converged` and `desired`, an `in_force` that
> is not `observed`;"

> §2.10: "`last_known`: `last_known` with no `observed`."

Each check names a member present. Two read otherwise in the fixture:
- **`in_force` absent.** "An `in_force` that is not `observed`" reads as
  one written and different. `reports.json` case 62 ("converged, nothing
  said in force") has none, and expects `in_force`: an absent one is
  "not `observed`" too.
- **`last_known: false`.** "`last_known` with no `observed`" reads as the
  member written. The description says "last_known true without
  observed", and case 9 reads `false` written out as absent. No case has
  `false` without `observed`.

**zk2py:** the fixture's, both. An absent `in_force` beside a converged
`desired` is a finding, and `last_known` counts only when true.

**Wording that would remove the guess:** §2.10, "or, with `converged`
and `desired`, no `in_force` or one that is not `observed`", and
"`last_known` true with no `observed`".

**Status at desired.v1 0.2:** resolved by 0.2, as zk2py read it: "with
`converged` and `desired`, no `in_force`, or one that is not `observed`",
and "`last_known` true with no `observed`". `reports.json` case 67 passed
unchanged.

### F-166 · ambiguity · desired.v1 0.1 §2.7 "Equal is JCS-equal": an integer past 2^53

> §2.7: "**Equal** is JCS-equal (RFC 8785) for a payload encoded as JSON
> … The comparison is conservative: two encodings of one value that it
> reads as unequal cost an act, never skip one."

> §2.4: "**A type MAY carry its own revision:** a top-level member named
> `revision` … an integer from 0 to 2^64−1."

> §2.7: "**A re-issue that should act again** changes the document: a type
> that wants identical content acted on again carries a `revision`"

RFC 8785 canonicalizes I-JSON, whose numbers are IEEE 754 doubles. Read
so, `{"revision": 9007199254740992}` and `{"revision":
9007199254740993}` (2^53 and 2^53 + 1) are one document: the second is
adopted, and the author's re-issue that moved its revision is never acted
on. That is the act the comparison claims never to skip. A strict RFC
8785 library, Python's `rfc8785` among them, refuses such an integer, and
the text says nothing for a document it cannot canonicalize: it reads as
strict JSON, so it is not "JSON that does not read". A `DesiredConfig`'s
revision and `tasking.v1`'s are JSON integers from 0, bounded by nothing
below 2^64. No fixture has one past 2^53.

**zk2py:** JCS bytes where both documents are in RFC 8785's domain.
Where it refuses an integer past ±(2^53−1) (`IntegerDomainError`), and
only then, equality by exact value, a boolean never a number, over two
values strict JSON reading can yield. Any other value RFC 8785 refuses (a
non-finite float, a string with no UTF-8 form, a key that is not a string,
a value that is not JSON) is no document read as strict JSON, and equals
nothing, itself included: it costs an act, never skips one
(`desired.jcs_equal`). Twelve readings pin both (review of PR #843).

**Wording that would remove the guess:** §2.7, "JCS-equal, a number
compared by its exact value: two documents that differ in an integer
beyond 2^53 are two", or a revision bounded at 2^53 − 1 for a JSON type.

**Status at desired.v1 0.2:** resolved by 0.2, partly against zk2py's
reading. §2.7: "**Every number compares by its exact value** (0.2), the value its
text denotes, never the double RFC 8785 reads it as". zk2py had compared
JCS bytes inside RFC 8785's domain, which read `0.1` and
`0.10000000000000000555` as one document; `order.json` case 76 pins two.
zk2py now reads a payload's text with every non-integer number as its
decimal (`strict_json(…, exact=True)`) and compares exact values
structurally, and no longer uses `rfc8785` here.

### F-167 · ambiguity · desired.v1 0.1 §2.10 "How a reader reads one": "every number finite" for an integer's text

> §2.10: "As `alarms.v1` §2.4 reads a document: strict JSON first (UTF-8,
> RFC 8259, nothing after the value but whitespace, no duplicate member at
> any depth, every number finite, every string with a UTF-8 form)"

> telemetry.v1 §2.6 (0.3): "**Finite** (0.3) is by rounding: a number's
> text, an integer's included, is finite when its exact value, rounded to
> the nearest double …, is not an infinity … `1e400`, 10^400 written out."

`alarms.v1` §2.4, which the text cites, never says what "finite" is for
an integer's text; `telemetry.v1` decided it for its own samples. Read as
`telemetry.v1` reads it, a report whose `revision` is 10^400 written out is
`json`. Read with integers exact, it passes the schema (an integer from 0)
and is `revision_range`. The report is unreadable either way, but
`reports.json` names the first finding, so two readers write two codes. A
member the schema does not name holding such a number makes the report
unreadable, or readable. zk2py's own `alarms` reader takes integers
exact.

**zk2py:** `telemetry.v1`'s rounding (`desired.strict_json`). A reading
pins the revision's case.

**Wording that would remove the guess:** §2.10, "every number finite as
`telemetry.v1` §2.6 reads it", or the rule stated in `alarms.v1` §2.4 for
both.

**Status at desired.v1 0.2:** resolved by 0.2, as zk2py read it:
"**Finite** (0.2) is by rounding, as `telemetry.v1` §2.6 reads it".
`reports.json` cases 68 to 71 and `order.json` case 77 passed unchanged.

### F-168 · ambiguity · desired.v1 0.1 §2.17 step 1: the kill switch across a constrained face

> §2.17 step 1: "across a constrained face, the deployment's word stands
> for the descriptor, and step 3"

> §2.17 step 2: "listing it unbound, the kill switch: **not asked**,
> `unbound`, but for "is a document in force?", which answers **no**,
> `unbound`."

> §2.12: "Its report is `{"source": "baseline"}`, with no key"

"Stands for the descriptor" says the deployment's word answers step 2's
questions, listed and bound; "and step 3" says the tool goes past step 2.
`conclusions.json`'s format has no input for the deployment's word, and
reads every target across a face at step 3. A role unbound across a face
then puts a report with no key, which matches no slice, so the tool
answers `unobservable`, `nothing_crossed`, for a target the deployment
disarmed: never `unbound`, and never "no" for "is a document in force?".

**zk2py:** the fixture's, step 2 not read across a face. A reading pins
it.

**Wording that would remove the guess:** §2.17 step 1, "across a
constrained face, the deployment's word answers step 2", with a
`conclusions.json` input for it and a case of a role unbound across a
face.

**Status at desired.v1 0.2:** resolved by 0.2, against zk2py's reading.
§2.17 step 2: "**Its descriptor,** or across a constrained face the
deployment's word for what it would say, listed and bound (0.2): not read,
or no word had, **unobservable**, `no_descriptor`". zk2py now reads
`Reading.deployment` across a face; `conclusions.json` cases 31 to 34 pin
it.

### F-169 · gap · desired.v1 0.1 §2.10 and §2.12: the kill switch's report among several roles

> §2.10: "The report of a role is the one whose `key` is that role's slice
> (§2.3), so one resource may carry the reports of several roles
> (ZenSight's `applied/{topic}`)."

> §2.12: "Its report is `{"source": "baseline"}`, with no key, put before
> its tokens like any other (§2.10)."

A report finds its role by its key. An unbound role has no slice and its
report no key, so in a resource carrying several roles' reports, two
unbound roles' reports are the same document, at members the text does
not name, and no reader can say whose each is. §2.17 never reads one
(step 2 answers from the descriptor first). The MUST to put it has no
reader the text describes. No fixture reads a report among several.

**zk2py:** nothing: no owner or tool of zk2py's puts or finds reports on
a bus yet.

**Wording that would remove the guess:** §2.12, which member of a shared
report resource an unbound role's report occupies (its role's name, or
the slice it would have), or "a report with no key belongs to no role a
reader asks about".

**Status at desired.v1 0.2:** resolved by 0.2, stated: "A report with no
key belongs to no role a reader asks about" (§2.10, §2.12). No fixture
reads it, and zk2py reads no such report.

### F-170 · gap · desired.v1 0.1 §2.2 "A declaration is invalid": a target parameter's type

> §2.2: "**Every parameter is a target parameter.** … **A declaration is
> invalid** for each of these, checked in this order" (six reasons, none
> about a parameter's type)

> core §2.2: "**Parameter types** are `string`, `uint` and `path` … A
> `uint` value is written in decimal without leading zeros"

A target parameter is filled with a system or a service name (§2.3),
which a `uint` parameter cannot hold unless the name is digits. §2.2's
closed list has no reason for a target parameter typed `uint`, and
`declarations.json`'s resources carry no `params`, so a reader cannot
tell. Core §2.2 lets types play no part in matching, so the slice's key
still resolves, with a value its type refuses.

**zk2py:** no reason, as the list says: such a declaration is declared.

**Wording that would remove the guess:** a seventh reason, "`not_string`:
a target parameter whose type is not `string`", or a sentence that the
type plays no part here.

**Status at desired.v1 0.2:** resolved by 0.2, against zk2py's reading: a
seventh reason, `not_string`, read only of a name that is a single
parameter. `declare` now takes the parameters' types; `declarations.json`'s
six new cases pin it.

### F-171 · contradiction · desired.v1 0.1 §2.16 "One document type": acting once per revision against once per version

> §2.16: "its acting once per revision in an instance, a restart acting
> again, is §2.7's once per version."

> config.v1 §2.18: "An owner acts on each revision of the document bound
> to it once in an instance, whatever the outcome"

> §2.7: "A target MUST adopt, without an act, an admitted version whose
> document is equal to that of the last version it acted on or adopted …
> Any other admitted version is acted on"

§2.7 tells two documents apart by their content; `config.v1` by their
revision. They agree when the revision moves, and when a document is put
again unchanged. They disagree when the author changes a
`DesiredConfig`'s `values` and keeps its `revision`: §2.7 acts on the new
document, and `config.v1` §2.18 has acted on that revision once. #840's
DB feeds a `config.v1` owner through this profile's reading, and one of
the two rules has to give way. No fixture has the case.

**zk2py:** nothing: zk2py's `config.v1` owner reads no document through
`desired.v1`.

**Wording that would remove the guess:** §2.16, which rule a `config.v1`
owner fed through this profile keeps, and `config.v1` §2.18 saying the
same, or a `DesiredConfig` whose values change requiring a new revision.

**Status at desired.v1 0.2:** resolved by `desired.v1` 0.2 §2.16 and
`config.v1` 0.4 §2.18: an owner fed through `desired.v1` acts by its §2.7,
and "a `DesiredConfig`'s author MUST move `revision` whenever `values`
change". zk2py's `config.v1` owner still reads no document through
`desired.v1`.

### F-172 · gap · desired.v1 0.1 §2.16 "Its report is its own": the mapping's codes and pointer

> §2.16: "its refusals are §2.7's refusals whole, with codes of its own
> (`reach`, `contract`, `sensitive`, `not_found`, `invalid_request`,
> `unavailable`), its `failed` is §2.7's"

> §2.16: "`refusal` `{code, group, parameter}` is `refusal` with that code
> and the pointer `/values/<group>/<parameter>`."

> config.v1 §3: "Desired refusal code | `reach`, `contract`, `sensitive`,
> `not_found`, `invalid_request`, `unavailable`; and for `failed`, `busy`,
> `device-refused`", and its `DesiredRefusal` is `{code, group,
> parameter?}`.

The list of `config.v1`'s codes leaves out the two its `failed` carries,
`busy` and `device-refused`, which §2.10's `apply` is not. The pointer is
stated for a refusal with a parameter only: a group's refusal (`reach`,
`contract`, `unavailable`, or `not_found` of a group) has none, and its
pointer, `/values/<group>` by the obvious reading, is not said. Neither
is RFC 6901's escaping of a name with `/` or `~`. A reader shows an
unknown code as written (§2.10), so nothing breaks, and no fixture reads
the mapping.

**zk2py:** nothing to implement: the mapping is a tool's display of
`config.v1`'s own report, which this profile's §5 does not read.

**Wording that would remove the guess:** §2.16, "codes of its own: …;
and for `failed`, `busy` and `device-refused`", and "the pointer
`/values/<group>`, then `/<parameter>` when the refusal names one, each
escaped as RFC 6901 says".

**Status at desired.v1 0.2:** resolved by `desired.v1` 0.2 §2.16 and
`config.v1` 0.4 §2.18: `busy` and `device-refused` listed for `failed`, and
"the pointer `/values/<group>`, then `/<parameter>` when the refusal names
one … each name escaped as RFC 6901 says: `~` as `~0`, then `/` as `~1`".

### F-173 · contradiction · desired.v1 0.1 §3, `desired.v1.json`: `observed`'s description

> `desired.v1.json`, `observed`: "The last version the target acted on in
> this instance, whatever the outcome (§2.7). Absent before its first
> act."

> §2.10: "`observed`: the last version the target acted on or adopted in
> this instance"

The published schema's description leaves adoption out, and the text and
`order.json` (cases 43 to 60) move `observed` to every adopted version.
The Report's own description says the same ("the last version it acted
on with that act's outcome"). Descriptions are annotations (core §7.3), so
no reader acts on them, but the file is the published artifact, and its
words disagree with the rule a reader of it is told. Mending them changes
the artifact's id, which §3 then publishes beside this one.

**zk2py:** §2.10's rule.

**Wording that would remove the guess:** in the file's next revision,
"The last version the target acted on or adopted in this instance".

**Status at desired.v1 0.2:** resolved by 0.2: the schema's revision
`sha256:43c95259…` says "acted on or adopted", published beside
`sha256:8c5ffce3…`. zk2py's `PUBLISHED` lists both; `schema.json` passed
with them.

### F-174 · contradiction · desired.v1 0.1 CHANGELOG "Stated", Evidence: `bindings.json`'s count

> CHANGELOG 0.1: "**Evidence:** `conformance/declarations.json` (37
> cases), `conformance/bindings.json` (20), `conformance/order.json` (64),
> `conformance/reports.json` (66), `conformance/conclusions.json` (31),
> `conformance/schema.json` (3)"

`bindings.json` holds 21 cases. The six files hold 222, and the entry's
counts add to 221. The count is informative and decides nothing.

**zk2py:** runs every case of the file.

**Status at desired.v1 0.2:** resolved by 0.2: 0.1's evidence line is
corrected as an erratum, and 0.2's counts (258) are the files'.

### F-175 · contradiction · desired.v1 0.1 scenarios.md §7 Expected 2 against §2.11: the restarted author's answer

> scenarios §7 step 2: "The author restarts from its record."

> scenarios §7 Expected 2: "The target sees the author's instance token,
> seeds, and the author answers v1 under `s1`: the same version,
> confirmed. The target acts on nothing, and puts its report without
> `last_known` (§2.9, §2.10)."

> §2.11: "A new instance of an author, by a restart or a re-mint … MUST
> put each document it keeps again, unchanged, from its new instance,
> before it answers a GET with it … Its GET answers then carry its own
> stamps (core S2) … Its session is a new one, its zid fresh (core §1.5):
> an instance never reuses another's."

An author that restarts from its record answers under its new instance's
stamps, so its answer can never be v1 under `s1`. That was the first
draft's design, with the zid reused, which the CHANGELOG's "Decided on
review" 1 dropped. Under 0.1 the target receives v1's document under a
new id: it is adopted (`order.json` case 59), not confirmed. `observed`
moves to the re-put's stamp and `last_known` goes. So the report's end
state is not §7's either, though no act is made. A runner that checks §7
as written fails a conforming author. No scenario then exercises §2.9's
confirmation on a bus, which only `order.json` cases 30 and 31 check.

**zk2py:** no harness this round. A runner would expect `adopted`, with
`observed` the new instance's stamp, `revision` 1.

**Wording that would remove the guess:** §7 Expected 2, "the author
answers v1's document under a stamp of its new instance: another id, the
same document, adopted (§2.7). The target acts on nothing, and its
report moves to that stamp without `last_known`". A section where the
same author instance answers after an archive was read, its GET silent
within the timeout or its link cut, would then show confirmation.

**Status at desired.v1 0.2:** resolved by 0.2: §7 Expected 2 now adopts the
restarted author's re-put under its new zid, and a new §10 shows
confirmation by the same author instance after a heal.

### F-176 · gap · desired.v1 0.1 scenarios.md §5 Expected 1: the baseline report against the target's first act

> scenarios §5 step 1: "G GETs `Rp` the moment S sees its new instance
> token."

> scenarios §5 Expected 1: "G's reply holds `{key: D, source: baseline}`
> under the new instance's stamp (§2.10) … The target seeds, acts on v1
> again"

> §2.5: "A target MUST seed at its start"; §2.10: "A target MUST put its
> first report of each role before its tokens …, so that a GET made when
> its instance token appears finds it"

The author holds v1 throughout §5, so the target's first seed draws it
at once. Nothing orders that seed after the target's tokens: core §8.2
orders an owner's declarations and tokens, and §2.5 says "at its start".
A target that seeds before its tokens, or soon after, and whose simulated
drive is quick, has put a converged report before G's GET reaches it, and
G reads that one. The expectation holds only if G wins a race the text
does not arrange. §1's step 1 has no such race, since its author starts
with no document.

**zk2py:** no harness this round. A runner would hold the author's reply
(the drive's act, or the author's queryable) until G's reply arrives, or
accept either report.

**Wording that would remove the guess:** §5's setup holding the target's
act until G has its reply, or §2.5 "a target seeds after its tokens",
which the text would then have to state.

**Status at desired.v1 0.2:** resolved by 0.2: §5's setup holds the
target's act from its start until G has its reply; the drive's simulation
can hold one. §2.5 keeps "at its start".

### F-177 · gap · desired.v1 0.1 scenarios.md "The contracts": `tasking.json` in prose

> scenarios.md: "The reference keeps them as
> `zenkey/tests/contracts/tasking.v1.toml` and `worker.v1.toml`, beside
> `tasking.json` … `tasking.json` defines one type, `Task`, an open object:
> `revision`, an integer from 0, required (the type's revision, §2.4);
> `mode`, one of `idle`, `run`, `halt`, required; `rate_hz`, an integer
> from 1 to 1000, required."

Both contracts are given in full, but the schema they list is described,
not given, and its file lives outside `spec/`. `telemetry.v1`'s scenarios
give `meter.json` byte for byte. A second implementation writes its own
`tasking.json`, whose bytes differ (its descriptions, its spelling of
"open", its member order), so its `Task`'s artifact id, `tasking.v1`'s
and `worker.v1`'s fingerprints, the tokens' `fp16` and every retrieval
differ from the reference's. Run alone, it can still pass §1 to §9. Run
against the owner example's `--desired`, its target and the reference's
author would hold two contracts of one name and minor.

**zk2py:** no harness this round.

**Wording that would remove the guess:** `tasking.json` given in full, as
`meter.json` is, or both contracts and their schema published under
`spec/profiles/desired/`.

**Status at desired.v1 0.2:** resolved by 0.2: `tasking.json` is given byte
for byte. zk2py's transcription of the three blocks loads with no finding
(W107 included), and gives `tasking.json` `sha256:b3da756f…`, `tasking.v1`
`sha256:a61f80ba…` and `worker.v1` `sha256:66e46fce…` ("At desired.v1
0.2").

### F-178 · gap · desired.v1 0.1 scenarios.md §1 Expected 3: who sees the target during the cut

> scenarios §1 step 3: "The test cuts the link between R2 and R1. The
> author puts v2. S, on R1, waits 2 s."

> scenarios §1 Expected 3: "The target's subscription receives nothing of
> v2, and its report is unchanged."

S is on R1, and the target and its report on R2, across the cut. S cannot
see the target's subscription, and sees no put on `Rp` made during the
cut, whether or not one was made: zenoh replays nothing at the heal.
Nothing in the section places an observer on R2's side. The target's own
log, a G on R2, or `Rp`'s stamp after the heal would each do.

**zk2py:** no harness this round. A runner would add a session on R2
holding a subscription to `Rp`.

**Wording that would remove the guess:** §1's conventions, "S2, a client
of R2, subscribes to `Rp`", and Expected 3 read through it.

**Status at desired.v1 0.2:** resolved by 0.2: S2, a client of R2,
subscribes to `Rp` and `Run` before anything starts, and §1 Expected 3 and
§10 read through it.

### F-179 · ambiguity · desired.v1 0.1 §2.11 and scenarios.md §4 step 3: a re-mint in one session

> §2.11: "A new instance of an author, by a restart or a re-mint (core
> §1.5, §8.1) … Its session is a new one, its zid fresh (core §1.5)"

> core §1.5: "It SHOULD do so in a new session, so that its timestamps'
> id changes too"; core §8.1: "An owner that starts a new instance id
> while running MUST declare the new descriptor and tokens first … then
> undeclare the old ones."

> scenarios §4 step 3: "The author's instance is re-minted (core §1.5),
> so it puts v3's content again from its new instance (§2.11)"; Expected
> 3: "The re-put carries v3's document under a later stamp"

Core lets a running owner re-mint in its own session, keeping its zid. Its
re-put then carries the same id and a later time, and §4's "a later
stamp" reads that way. §2.11 states the session is new, so the re-put is
another id. Both are adopted, so the target's report is the same, but a
runner cannot tell which stamp to expect, and §2.11's "MUST put … before
it answers a GET with it" is ambiguous for a session whose queryables
answered throughout.

**zk2py:** no harness this round. A runner would accept the re-put under
either id.

**Wording that would remove the guess:** §2.11, "Its session is new, its
zid fresh, unless it re-mints in its own session (core §8.1), whose re-put
carries its zid and a later time", and §4 step 3 saying which it runs.

**Status at desired.v1 0.2:** resolved by 0.2: §2.11 "**Its zid** (0.2)"
states both cases, and §4 step 3 re-mints in its own session, its Expected
3 "a later stamp of the same id".

### F-180 · gap · desired.v1 0.1 scenarios.md §9 Expected: questions left unanswered

> scenarios §9 Expected 2: "Converged: **no**, `refused`. Refused: **yes**,
> `refused`. In force: **yes**, `desired`." 3: "… In force: **yes**,
> `partial`." 4: "Converged: **not asked**, `author_absent`. Last-known:
> **yes**, `last_known`."

§9's setup says T "asks §5's four questions", and Expected 1 and 5 give
all four. Expected 2 and 3 leave out last-known, and 4 leaves out
refused and in force. The text answers them (last-known **no**,
`current`; refused **no**, `converged`; in force **yes**, `desired`), but
a scenario's expectations are what a tool is checked against, and a
check left out is not made.

**zk2py:** no harness this round. A runner would check all four, as the
text answers them.

**Wording that would remove the guess:** §9 Expected 2 to 4 with the
missing answers.

**Status at desired.v1 0.2:** resolved by 0.2: §9 Expected 2 to 4 give all
four answers.

**Counts at desired.v1 0.1 and core 0.36:** 180 entries, F-161 to F-180
open, every one before them resolved.

## At desired.v1 0.2 (#609)

`desired.v1` 0.2 answers round 52's twenty findings (its CHANGELOG says
what each became), with `config.v1` 0.4, which takes F-171's and F-172's
side of them. Core is 0.36, unchanged. zk2py read both texts cold and
implemented each 0.2 rule from the text before running the fixtures:
- **§2.6 step 4, "never an older version of an id it remembers"**:
  `TargetRole.remembered`, per id the latest time acted on (recorded when
  the act starts, §2.7) or adopted, for the 16 most recent such ids
  (`REMEMBERED_IDS`), an id moved to the most recent at each act or
  adoption, the oldest forgotten; a restart forgets every one. A sample
  is judged against the version last acted on or adopted, then the
  remembered ids, then the one held waiting.
- **§2.7, every number by its exact value**: a payload's text read as
  strict JSON with every non-integer number as an `Exact`, a normalised
  (sign, significant digits, exponent) form computed from its text, then
  compared structurally, members in any order, an `int` and a `float`
  normalised the same way. The cost is bounded by the numbers' texts,
  never their exponents' values (review of PR #845: `Decimal` refused an
  exponent past about 10^18 with `InvalidOperation`, which the reader did
  not catch). An exponent past 25 digits, whose value's double is 0 or an
  infinity, is kept as text, so one value written two ways there reads as
  two, which costs an act. RFC 8785's library is no longer used for
  this.
- **§2.10, finite by rounding and at most 256 deep**, for a report and a
  document, counted as `telemetry.v1` counts it.
- **§2.2, `not_string`**: `declare` takes the parameters' types and reads
  the type of a name that is a single parameter.
- **§2.17 step 2, the deployment's word** across a face, `no_descriptor`
  without one; the descriptor is not read there.
- **§3**: 0.2's id `sha256:43c95259…` published beside 0.1's.

**`just py-conformance`: 2652 of 2652** (2637 before review of PR
#845 added 15 readings of exponents and of the finite edge). At the start
of the round,
round 52's code stopped the `desired` family with exit 2 at `order.json`'s
new `text` input, which it did not read; its readings of F-163, F-166,
F-168 and F-170 would have failed the cases 0.2 added for them.

| Family | Before | After |
|---|---|---|
| desired | could not run (exit 2) | 341 of 341 |
| readings | 246 of 246 | 276 of 276 |
| the 21 others | 2035 of 2035 | 2035 of 2035 |

The `desired` family's six files hold 258 cases: `schema.json` 3 (and
five checks of its header against zk2py's tables), `declarations.json`
43, `bindings.json` 25, `order.json` 78 (and 78 read-backs of its
reports), `reports.json` 74, `conclusions.json` 35. Every count matches
the CHANGELOG's. `order.json` case 72 fails with 15 ids remembered, so
the memory's floor is pinned, and its ceiling is not, as the text says.

The `readings` now hold 73 of `desired.v1`. Fifteen were added on review
of PR #845: exponents of 10^9, past 10^22 and of 100,000 digits, each
compared in under half a second, a leading-zero exponent, `-0.0e5`, `1e2`
against `100`, a 4301-digit integer, and the finite edge 2^1024 − 2^970
written as an integer, with a fraction, as `1.7976931348623159e308` and
as `-1e309`, each refused and its neighbour read. Of the rest, fifteen
are new and three changed for 0.2: the archive's stale old-id version is older; the 17th id
forgets the first, whose stale value is then acted on, the residual the
text accepts; the 16 most recent are kept; a remembered id's latest time
again is the same version; a restart forgets every id; `-0` and `0`,
`1e1` and `10.0` and `100e-1` and `10` are one value each; two decimals
apart at the 31st digit are two; 256 deep compares by value, 257 deep
byte for byte; a report 300 deep is `json`; F-181's two and F-184's one
(below); and the deployment's word against a descriptor read across a
face, and ignored when present.

**Round 52's findings at 0.2.** All twenty are resolved, each with a
status line under its entry ("At desired.v1 0.1").

| Finding | Status | zk2py |
|---|---|---|
| F-161 `wrong_self` | resolved, as zk2py read it | unchanged |
| F-162 running act's version first | resolved, as zk2py read it | unchanged |
| F-163 never older, per id | resolved, against zk2py's reading | 16 remembered ids |
| F-164 the id 0 | resolved, stated | unchanged |
| F-165 an absent member | resolved, as zk2py read it | unchanged |
| F-166 numbers past 2^53 | resolved, partly against zk2py's reading | exact values everywhere |
| F-167 finite by rounding | resolved, as zk2py read it | unchanged |
| F-168 the kill switch across a face | resolved, against zk2py's reading | the deployment's word |
| F-169 the keyless report | resolved, stated | nothing to implement |
| F-170 a target parameter's type | resolved, against zk2py's reading | `not_string` |
| F-171 once per revision | resolved by `config.v1` 0.4 | nothing to implement |
| F-172 the mapping | resolved by both texts | nothing to implement |
| F-173 the schema's description | resolved, a second id | both ids published |
| F-174 the count | resolved, an erratum | unchanged |
| F-175 §7's confirmation | resolved: adopted, and §10 | read for sufficiency |
| F-176 §5's race | resolved: the act held | read for sufficiency |
| F-177 `tasking.json` | resolved: given in full | transcribed, loads clean |
| F-178 the cut's observer | resolved: S2 | read for sufficiency |
| F-179 a re-mint's zid | resolved, both stated | read for sufficiency |
| F-180 §9's answers | resolved: all four | read for sufficiency |

**The scenarios, read again for sufficiency.** No harness was built:
the runtime owner is #840's DC.
- **`tasking.json` in full (F-177).** zk2py took the three blocks of
  "The contracts", stripped the list's two-space indent, and placed the
  files as the reference keeps them, with the profile's file at the path
  `worker.v1` lists. Both contracts load with no finding, W107 included.
  `worker.v1`'s `desired/task` is the profile's Report by type identity
  (0.2's id), and `tasking.v1`'s template declares `[system, service]`.
  The ids it computes: `tasking.json` `sha256:b3da756f…`, `tasking.v1`
  `sha256:a61f80badbd990e20130f592c03038f0bdd3ab0e8ba3a855040560057b27a645`,
  `worker.v1`
  `sha256:66e46fce140ea39b8b939ca17c1a7d607108aee7147dc12a5608b6cdfd876454`.
  The text gives none to compare with, as no profile's scenarios do; a
  runner against the owner example's `--desired` would compare them
  there.
- **§5's hold (F-176)** is runnable: the drive holds an act from its
  start, and G's GET then finds the baseline report. A black-box runner
  arms the hold in the new process, which DC's `--desired` interface
  will have to allow.
- **§7 Expected 2 (F-175)** agrees with §2.11 and `order.json` case 59:
  the restarted author's re-put under its new zid is adopted.
- **§10** shows §2.9's confirmation: a target restarted during a cut
  reads the archive on its side, then, after the heal, the same author
  instance answers `s1`, confirmed. Its steps and observations need
  nothing the text does not give, but no rule cites it (F-183).
- **§1's S2, §4's re-mint in its own session and §9's four answers**
  (F-178 to F-180) are runnable as written.

### F-181 · ambiguity · desired.v1 0.2 §2.10 "How a reader reads one": a report's fractional number, exact or a double

> §2.10: "Then the schema, an integral number read as an integer (`3.0` is
> `3`), a member the schema does not name ignored; a stamp in the form of
> §2.4, its time at most 2^64−1 and its id at most 128 bits; a revision
> at most 2^64−1."

> §2.7: "**Every number compares by its exact value** (0.2), the value its
> text denotes, never the double RFC 8785 reads it as"

> telemetry.v1 §2.6: "**A float** (JSON text with a fraction or an
> exponent, a CBOR float) is the IEEE 754 double it denotes, the nearest
> one for JSON text."

0.2 says what a document's number is (its exact value) and when a
report's number is finite (by rounding), but not what a report's number
is once it reads. A revision written with a fraction or an exponent past
2^53 differs by the reading:
- `18446744073709551615.0` is 2^64 − 1 exactly, a revision in range, or
  the double 2^64, `revision_range`;
- `in_force`'s `9007199254740993` beside `observed`'s
  `9007199254740993.0` is one revision exactly, and two by the double
  (2^53): an `in_force` finding with `converged` and `desired`.

`reports.json` has no such case, and zk2py read every case alike either
way: it substituted an exact reader and ran the file again.

**zk2py:** the double, as `telemetry.v1` §2.6 reads a float's text,
which §2.10 already follows for finiteness. Two readings pin it.

**Wording that would remove the guess:** §2.10, "a number with a
fraction or an exponent is the double it denotes, as `telemetry.v1` §2.6
reads it", or "its exact value, as §2.7 reads it", with a `reports.json`
case.

**Status at desired.v1 0.3:** resolved by desired.v1 0.3, against zk2py's
reading. §2.10: "every number read by its exact value, as §2.7 compares a
document's (0.3, F-181): a number with a zero fraction is that integer, so
`3.0` is `3` and `18446744073709551615.0` is 2^64−1". zk2py had read the
double. It now reads a report's bytes with every number exact
(`exact_json`), and a document's revision too (§2.4, `revision_of`).
`reports.json`'s five new cases pin it.

### F-182 · contradiction · config.v1 0.4 §2.18: "(text 0.4)" after `desired.v1`

> config.v1 §2.18: "`config.v1` does not define desired state;
> [`desired.v1`](../desired/v1.md) does (text 0.4), and its §2.16 states
> this profile's rules as those of one document type."

Placed after the link, "(text 0.4)" reads as `desired.v1`'s text 0.4,
which does not exist: `desired.v1` is at 0.2. The marker is
`config.v1`'s own (its 0.4 added the sentence), as its other markers are,
"(text 0.3, F-114)", "(text 0.4, F-171)", but each of those follows the
rule it dates, not another profile's name.

**zk2py:** nothing to implement.

**Wording that would remove the guess:** "`desired.v1` does, and its
§2.16 (text 0.4 here) states …", or "(`desired.v1` text 0.2)".

**Status at desired.v1 0.3:** resolved by config.v1 0.5: "text 0.5, F-182:
this marker is this text's version, never `desired.v1`'s".

### F-183 · gap · desired.v1 0.2 §2.9 and core Appendix C: scenario §10 cited by no rule

> scenarios.md §10: "Last-known confirmed by its author (§2.9, 0.2)"

> §2.9: "**Confirmed by the author.** When its author's answer, or a live
> sample, later carries the same version, the target holds it as its
> author's: it does not act again, and its report's `last_known` goes.
> `[F: order.json]`"

> core Appendix C: "`profiles/desired/scenarios.md` §1, §3, §6, §7 | …
> S5, S6 last-known from an archive, the owner absent (0.36)"

§10 is the one scenario that shows §2.9's confirmation, F-175's answer,
but no rule of the text tags it `[Sc: §10]`: §2.9's bullet cites
`order.json` alone, and its "From an archive" and "Reported as
last-known" cite §7. Core Appendix C's index lists §1, §3, §6 and §7,
while §10 observes S6's other branch, an owner that gave no reply within
the GET's timeout while it ran, and §4.2's seed when its instance token
appears again after a cut. A reader following the tags, or the index,
never reaches §10. Both are indexes, and decide nothing.

**zk2py:** nothing to implement.

**Wording that would remove the guess:** `[Sc: §10]` on §2.9's
"Confirmed by the author" (and its "From an archive"), and §10 in core
Appendix C's row, "S6 an owner silent within the timeout".

**Status at desired.v1 0.3:** resolved by desired.v1 0.3: §2.9's "Confirmed
by the author" and "From an archive" cite `[Sc: §10]`, and core Appendix
C's row lists §10, an index entry that leaves core at 0.37.

### F-184 · ambiguity · desired.v1 0.2 §2.6 step 3: what "holds waiting" confirms

> §2.6 step 3: "equal: **the same version** (`same_version`), received
> again by another path or another seed, never acted on again; its
> author's own sample of a version the target holds from an archive, or
> holds waiting while an act runs, **confirms** it (`confirmed`, §2.9);"

> `order.json`, description: "the author's own sample of a version held,
> or held waiting, from an archive is confirmed instead"

The text's "or holds waiting while an act runs" can be read without
"from an archive": then the author's own sample of any version held
waiting, the author's own included, confirms it. The description puts
"from an archive" on both, and §2.9 confirms only a version read from an
archive. `order.json` case 66 holds an archive's version waiting; no case
re-delivers an author's version held waiting, which is `same_version` by
the description and `confirmed` by the text read literally.

**zk2py:** the description's: an author's version held waiting and
delivered again is the same version. A reading pins it.

**Wording that would remove the guess:** §2.6 step 3, "its author's own
sample of a version the target holds from an archive, or holds waiting
from an archive while an act runs, confirms it".

**Status at desired.v1 0.3:** resolved by desired.v1 0.3, as zk2py read it:
step 3 now says "holds waiting from an archive", and "any other delivery of
a version held waiting, the author's own version included, is the same
version". `order.json`'s two new cases passed unchanged.

**Counts at desired.v1 0.2, config.v1 0.4 and core 0.36:** 184 entries,
F-181 to F-184 open, every one before them resolved.

## At core 0.37 (#609)

Core 0.37 says what a complete presence read is (§8.1, §2.7, Appendix
B). It adds three rules:
- a read through a session cut off is not complete, and SHOULD NOT be
  made;
- absence is established only by a complete read, so a withdrawal heard
  live is a reason to read again;
- a read that may conclude absence goes on a session with no liveliness
  subscriber, beside a **watch session** that holds them, with one round
  trip there before the seed.

`scenarios/presence.md` §7 to §9 are new. No fixture changed, and the
profiles keep their texts. zk2py read §8.1 and Appendix B cold, and the
two files of `docs/zk2/upstream/` that Appendix B cites. It implemented
the rules from the text:
- **`zk2py.live`.** `list_presence` makes no read through a session that
  holds no transport, and none on a session marked by a liveliness
  subscriber zk2py declared. It reports each **not made**, a third kind
  (`Presence.kind`). It checks the transports again when the read ends
  (F-185).
- **The watch session.** `declare_liveliness_subscriber` marks a session
  for its life. `watch_session` opens a reader's watch session the way
  `open_client` opened the reader. `settle` is the round trip.
  `watch_or_beside` moves a reader's subscriber, and `AlarmsReader`,
  `budget.Window`, the telemetry reader, and the alarms, health and
  hostid runners' token watches use it.
- **`zk2py.presence`.** `PresenceView` is session-free. Its holders are an
  address's instance tokens, an interface's tokens and a member's epochs.
  A withdrawal that leaves one with nothing held makes it unobservable,
  never down, and owes a read; one read asked answers the burst. A
  make-before-break re-mint owes none. A complete read showing none makes
  it down. A read not made, or possibly incomplete, leaves it unconfirmed,
  and the read still owed. `PresenceFollower` is the live half. It reads
  0.1 s after a withdrawal, the reference's interval, which the CHANGELOG
  states and the text leaves at "soon", and retries a read not made every
  0.2 s.

**What zenoh-python 1.10.1 lets it do.** Everything the rules ask:
- `Session.info.transports()` is there, and its stub does not mark it
  unstable, unlike Rust's. It was empty within 0.05 s of a cut, and filled
  again within about 1 s of a heal, by the client's own retry.
- A transport events listener is there too (`budget.StallWatch` uses one).
- A liveliness GET with a callback handler, a liveliness subscriber with
  or without history, and a second client session are all stable API.

What it does not let zk2py do:
- **Mark a session itself.** A `zenoh.Session` takes no attribute and no
  weak reference. zk2py's registry therefore holds each marked session by
  its id, and keeps a strong reference, so that the id is never reused.
- **Open a watch session for a session it did not open.** A session's
  configuration cannot be read back. `budget_scenarios.client` opens its
  sessions through its own configuration, with a lease, so its windows
  keep their subscriber beside their reads, as before 0.37.

**`just py-conformance`: 2680 of 2680.** Before this round, 2652 passed;
`readings` gained 28 cases, of the view, of a read's kinds, and of
`list_presence`'s guards on fake sessions, and nothing else moved.

**On review of PR #848** zk2py tightened four things:
- **The registry.** Closed sessions are pruned as the registry is touched,
  and a closed reading session takes its watch session with it.
  `forget` and `close` drop a session's entries. Every lookup checks that
  the stored session *is* the one asked about. Without that check, a pruned
  entry's id reused by a new session handed it the previous section's
  watch session, which the alarms and telemetry runners showed at once.
- **A transports accessor that fails,** before the read or at its end,
  leaves the read possibly incomplete, with that reason, never complete.
- **The follower** reads its state under its lock. It retries a read
  that settles nothing after 0.2 s, doubling to a 5 s cap and back after
  a complete one, and survives a read that raises. A seed whose round
  trip did not come back is folded as possibly incomplete.
  `watch_or_beside` tries the round trip three times and says whether it
  came back. The readers that use it take a failed one as a start not
  shown complete.
- **The fold** lets each key's latest move since the read was asked
  decide it: put, deleted and put again is held. A withdrawal heard back
  owes no read.

The runners pass again after these changes: telemetry 57/57, alarms
`--quick` 78/78 (one not run), hostid 34/34, health §1, and
`presence_scenarios` 26/26 three times, now polling instead of sleeping.

**`python -m zk2py.presence_scenarios`: 26 of 26**, three runs. Each
section observes zenoh as the scenario says, then zk2py's reader:
- **§7.** Cut, the GET is answered at once: the final reply, no token, no
  error reply, in under a millisecond. R1's own read holds the token.
  zk2py's read is not made, cut off. After the heal, it holds the token.
- **§8.** On the cut, the watch session's subscriber hears the token
  withdrawn, and R1 still holds it. zk2py's follower reads the owner
  unobservable (withdrawn), never down, and its re-reads are not made
  while cut off. On the heal, the subscriber hears the token again, and
  the follower reads up. After a clean stop, the follower reads down,
  and only from its complete re-read.
- **§9.** A GET made at once beside a history subscriber completes without
  the token, in every run, as Appendix B measured. The tool's GET holds it,
  and zk2py's reader refuses a read on a marked session.
- **The follower** (zk2py's own). Three owners withdrawn at once take one
  read, then each is down. A make-before-break re-mint takes no read, and
  the owner stays up.
- **F-185.** A read in flight when the link closes comes back at once with
  no token, and zk2py's end-of-read check reads it not complete.

After the move to watch sessions, the converted runners pass against
zk2py's owner:
- `telemetry_scenarios`: 57 of 57;
- `hostid_scenarios`: 34 of 34;
- `health_scenarios`: every check of §1 to §8;
- `alarms_scenarios --quick`: 78 of 78, one not run.

`live_interop`'s budget run was not rerun: it needs the Rust owner
example, which cargo builds.

**The other profiles against rule (b).** No profile's verdict concludes
absence from a withdrawal:
- the presence premises of alarms.v1 §2.14 step 1, health.v1 §2.11,
  config.v1 §5 and desired.v1 §2.17 are complete reads;
- alarms.v1 §2.15 reads a withdrawal as a gap in its window, which is
  unobservable, not absence;
- telemetry.v1 §2.9 reads one as no evidence of an epoch, which is
  unobservable too;
- config.v1 §2.12's late confirm reads "the instance token it knew is
  gone, and another is there", which is positive evidence.

Two rules word absence loosely, and one premise has no input for a read
not made (F-187, F-191).

**The scenarios, read for sufficiency.** Each of §7 to §9 runs from its
text alone. But §8 and §9 observe zenoh more than the reader (F-188,
F-189), and §7 cuts no read in flight (F-185).

### F-185 · gap · core 0.37 §8.1 "A read through a session cut off": a transport that closes during the read

> core §8.1: "A reader SHOULD NOT make it: before a read, it checks that its
> session holds a transport to another node (in zenoh 1.10.1,
> `Session::info().transports()`, unstable, §0), and reports a read not
> made, with that reason, as it reports one possibly incomplete."

> core §8.1: "Such a read is **not complete, whatever its replies say.**"

The text checks before the read only. zk2py measured a read in flight when
the session's last transport closes. Its link was stalled, the read was
asked, and the link was cut 0.3 s later. It came back at the cut, with the
final reply, no token and no error reply, in 3 of 3 runs, and again in the
runner's check. A transport was held when it was asked, so the check
before passes it, and a reader reads absence. The answer came through a
session that by then reached no node, and the rule calls that not
complete. A reader can only know by looking again: at the read's end
(the transports empty then, since the answer comes at once at the close),
or by a transport event during it. §7 cuts the link before its read, so
it does not show the case.

**zk2py:** checks before and after (`list_presence`). A read whose
session holds no transport when it ends is not complete
(`cut_off_during`), and its tokens are kept as seen. The runner's F-185
check pins it.

**Wording that would remove the guess:** §8.1, "before a read, and again
when it ends, it checks …; a read during which its session's last
transport closed is not complete either", and a step of §7 that cuts the
link while a read is in flight.

**Status at core 0.38:** resolved by core 0.38: §8.1 "A read cut off
while it runs" checks again when the read ends, by the session's links
(`Session::info().links()`, never `transports()`), and reads such a read
possibly incomplete, cut off during it, its tokens seen; §7 gains step 5.
zk2py checks the links before and after (zenoh-python 1.10.1 has
`Session.info.links()`), and step 5 passes 10 of 10. In zenoh-python
`transports()` had dropped the closing transport by the read's end in all
10 (the reference measured 4 of 10 still listed), so the two accessors
agree here, and the links are what zk2py reads.

### F-186 · contradiction · scenarios/presence.md §4 against core 0.37 §8.1 and Appendix B: a GET beside a subscriber, expected whole

> presence.md §4: "**Steps.** A session that holds a liveliness subscriber
> issues a liveliness GET, under each pairing of the GET's handler and the
> subscriber's."

> presence.md §4, Expected: "With callbacks for both, the GET completes
> with every token."

> core §8.1: "a liveliness GET from a session that holds a liveliness
> subscriber can complete without live tokens the subscriber covers: the
> final reply and no error reply, the tokens missing. It happens with
> history or without"

§4 is unchanged at 0.37. It still expects a GET beside a subscriber to
hold every token, which Appendix B now measures failing, every time for
a history subscriber declared just before. §4 names neither the
subscriber's selector nor its history, nor the order of the tokens and
the subscriber. It holds in Appendix B's one safe cell: a subscriber
without history, declared after every token ("B tokens-first", 0 of
5,760). zk2py's run of §4 (`live_interop`, `_scale_check`) holds a
non-history subscriber declared after its holder declared the 2,000
tokens, which reach the router over about 3 s, and polls its GET until
it lists every one: a miss there delays the pass, and never fails it.
Read with 0.37,
§4 checks a rule (§8.1's handler rule) on the one read 0.37 says not to
make, and gives no read made that way a verdict.

**zk2py:** its §4 run unchanged, a measurement of the handler rule. The
GETs it reads presence by are on sessions with no subscriber.

**Wording that would remove the guess:** §4, "a session that holds a
liveliness subscriber without history, declared after the tokens, …
(Appendix B: any other subscriber can make the GET miss them)", or §4
moved to the watch session's round trip, whose answer is discarded.

**Status at core 0.38:** resolved by core 0.38: §4 is the watch
session's round trip, "its answer discarded … so this section counts no
token", at 1,200 tokens and a 5 s wait. zk2py runs it: callbacks for both,
0.14 s; a drained handler beside a callback subscriber, 0.05 s; a
callback round trip beside a bounded subscriber nobody drains, ended at
its timeout with an error reply. The bounded GET beside it is not run: it
hangs, as both implementations found.

### F-187 · ambiguity · desired.v1 0.2 §2.5 and alarms.v1 0.8 §2.7 against core 0.37 §8.1: "token gone"

> desired.v1 §2.5: "**The author absent.** With its author's instance token
> gone, or its GET silent within the timeout, the target MAY read an
> archive's last-known instead (§2.9, core S6)."

> alarms.v1 §2.7: "**An absent owner's alarms are last-known.** With its
> tokens gone, its alarms are presence's word and an archive's last-known
> (core S6), never firing now (§2.14)."

> core §8.1: "A reader MUST NOT conclude absence from a withdrawal alone."

> core S6: "A consumer turns to an archive only when the owner gave no
> reply within the GET's timeout, or presence shows it absent."

"Gone" reads as heard withdrawn, the one thing 0.37 says is never absence
by itself. A target whose own router restarts hears its author's token
withdrawn and turns to an archive on it. That is S6's turn without its
premise, since presence has shown nothing absent. alarms.v1's §2.14 step
1, which its CHANGELOG cites, makes its verdict a complete read, so only
§2.7's sentence is loose there. desired.v1's §2.5 rule is the target's
own act, and nothing else in desired.v1 binds it to a read.

**zk2py:** no target on a bus yet. Its reader would turn to an archive
only on a complete read without the token, or on a GET silent within the
timeout.

**Wording that would remove the guess:** "With its author absent, a
complete read without its instance token (core §8.1), or its GET silent
…", and the same in alarms.v1 §2.7.

**Status at desired.v1 0.3:** its desired.v1 half is resolved by desired.v1
0.3. §2.5, §2.9, §2.17 and §5 now read the author's absence as "a complete
read without its instance token (core §8.1)", and add "never on a
withdrawal heard live, nor through a session cut off". Its alarms.v1 half
is open: alarms.v1 §2.7 still says "With its tokens gone", its text 0.8
unchanged.

**Status at core 0.38:** its `alarms.v1` half is resolved too: alarms.v1
§2.7 now reads "With its owner absent — a complete read without its
instance token (core §8.1), never a withdrawal heard live alone", its
text 0.8 unchanged (its CHANGELOG, "0.8, read against core 0.38"). Both
halves resolved.

### F-188 · gap · scenarios/presence.md §8: the reader's verdict during the cut is not observed

> presence.md §8 Expected 2: "On the cut, the subscriber hears the owner's
> instance token withdrawn, and R1's own session still reads it: the owner
> never left. On the heal, the subscriber hears it again, and the tool's
> read holds it. A reader that concluded absence from the withdrawal would
> have read a live owner down"

Every observation of §8 is of zenoh: what the subscriber hears, what R1
holds, what the tool's read holds after the heal. None is of the reader's
verdict. The sentence on a reader that concluded absence describes a
failure, but no step asks the reader between the cut and the heal. So an
implementation that reads the owner down from the withdrawal passes §8.
Nor does any step observe §8.1's other four points:
- one read for a burst;
- a make-before-break re-mint owing none;
- a re-read not made while cut off, leaving the withdrawal unconfirmed;
- step 3's absence said only once the read after the withdrawal completes.

**zk2py:** its runner asks its follower at each of those points. It also
runs a burst and a re-mint of its own (the "follower" section).

**Wording that would remove the guess:** §8 Expected 2, "between the cut
and the heal, the reader reports the owner unobservable, never down, and
its re-read not made", and steps for a burst and a re-mint.

**Status at core 0.38:** resolved by core 0.38: §8's Expected ask the
reader under test, and its steps 4 and 5 add the burst and the re-mint.
zk2py's follower passes every step through the cut link: unobservable and
withdrawn, its re-read not made, while cut; up on the heal; down only from
the complete read after the clean stop; one read for three; none for the
re-mint.

### F-189 · gap · scenarios/presence.md §9: the reader's own placement is not observed

> presence.md §9 Expected 3: "A reader makes the read that establishes
> absence on a session like the tool's (core §8.1, "The read that
> establishes absence"): the reference refuses a liveliness read on a
> session that holds a liveliness subscriber it declared, and reports it
> not made."

Steps 2 and 3 are GETs the test makes, so they show zenoh's miss and its
cure. A reader that holds its subscriber and makes its own reads on one
session passes them. §8.1's round trip on the watch session before the
seed is in no step either. Its case, a token withdrawn between the seed's
answer and the router's reading of the subscriber, is never made to
happen, so a reader that leaves it out passes as well.

**zk2py:** its runner checks its own refusal on a marked session. The
round trip is in its follower and its readers (`settle`), and nothing
observes it.

**Wording that would remove the guess:** a step in which the reader under
test follows and seeds presence, observed to make no liveliness read on its
watch session but the round trip, and one that withdraws a token right
after the seed is answered.

**Status at core 0.38:** resolved by core 0.38: §9 steps 4 and 5
observe the reader. zk2py counts each session's liveliness GETs: the
watch session makes the round trip and no other, the reading session every
read, and a read asked on the watch session is not made. Step 5, the
watch link stalled 150 ms from before the subscribers and the token
withdrawn as the seed is answered: the follower heard it go and read the
owner down in 10 of 10; the control, a follower that skips the round
trip, in 0 of 10.

### F-190 · ambiguity · core 0.37 §8.1 "The read that establishes absence": "holds no liveliness subscriber", and one undeclared

> core §8.1: "It happens with history or without, and by zenoh's code it
> lasts after the subscriber is undeclared (Appendix B)."

> core §8.1: "A reader SHOULD make every liveliness read it may conclude
> absence from … on a session that holds no liveliness subscriber of any
> kind"

"Holds no" is the present tense. A session whose subscriber was declared
and then undeclared holds none, yet the sentence before says it stays
exposed. The reference marks a session "for its life", as the CHANGELOG
says, which is the stricter reading.

**zk2py:** the CHANGELOG's, a session marked for its life.

**Wording that would remove the guess:** "on a session that has never held
a liveliness subscriber of any kind".

**Status at core 0.38:** resolved by core 0.38: §8.1 reads "has never
held a liveliness subscriber of any kind", and §9 Expected 3 "a session
that has held one". zk2py marked a session for its life already.

### F-191 · gap · the profiles' presence premises against core 0.37: a read not made

> core CHANGELOG 0.37: "**Possibly incomplete** reads (a timeout, an error
> reply) keep their meaning. A read not made is a third kind, never folded
> into either."

> core §8.1: "… reports a read not made, with that reason, as it reports
> one possibly incomplete. What a read not made would have established is
> unobservable."

> desired.v1 `conclusions.json`: "incomplete: not seen in a read possibly
> incomplete or refused"

A tool's question goes through a profile's premises, and every profile
names three readings: present, absent, and a read possibly incomplete or
refused:
- alarms.v1 §2.14 step 1, and its `conclusions.json` presence;
- health.v1 §2.11;
- config.v1 §5;
- desired.v1 §2.17 steps 1 and 6, and its `conclusions.json` input;
- telemetry.v1 §2.9, a view that stands on a complete read.

None names a read not made. A tool must give one a profile's unobservable
reason, and the only one there is "presence possibly incomplete" (for
desired.v1, `presence_incomplete`). That folds the third kind into the
second, which the core says never to do. The answer is unobservable either
way, but the reason misstates the read.

**zk2py:** a read not made goes to each profile as `incomplete`, its
`Presence.kind` and reason kept beside it.

**Wording that would remove the guess:** each premise, "a read possibly
incomplete, refused, or not made (core §8.1)", or a core sentence that a
profile's possibly-incomplete reason covers a read not made.

**Status at desired.v1 0.3:** its desired.v1 half is resolved by desired.v1
0.3: §2.17 steps 1 and 6 count "a read not made (core §8.1, 0.37: a session
cut off)" beside the possibly incomplete, under `presence_incomplete`. The
other profiles' premises are unchanged, so the rest stays open.

**Status at core 0.38:** resolved by core 0.38's premise rule (§8.1, "A
profile's premise"): a read not made is read as the profile's possibly
incomplete premise reads it, unobservable, the profile's reason standing
and the read's own reported beside it. No profile text changed, and none
needed to: zk2py already read a read not made so, its `Presence.kind` and
reason kept beside the profile's. `redundancy.v1` §2.15 states the same
rule for its own questions.

**Counts at core 0.37:** 191 entries. F-181 to F-191 are open, F-181 to
F-184 against desired.v1 0.2, and every one before them is resolved.

## At desired.v1 0.3 (#609)

`desired.v1` 0.3 answers round 53's findings and F-187's `desired.v1`
half, and says what building the reference's runtime showed: how a target
hears its author appear, a seed made again at an appearance, the archive
read only on absence, `invalid` for a schema's refusal, an act whose end is
unknown, a re-mint not a restart, the author's record and a paced re-put,
and what a delete reverts `config.v1` to. `config.v1` 0.5 states that
revert, and moves F-182's marker. zk2py read both cold, implemented each
change from the text, and ran the profile live for the first time.

**The static half.** §2.10 now reads a report's numbers by their exact
values (`exact_json` over strict JSON's `Exact`). §2.4 reads a document's
revision the same way (`revision_of`). At the start of the round,
`desired` passed 348 of 350: `reports.json` cases 74 and 77, F-181's
answer, which zk2py had read as doubles.

**`just py-conformance`: 2709 of 2709.** `desired` passes 350 of 350,
the 265 cases with five header checks and 80 read-backs. (After the
round, a 266th case joined `reports.json`, a revision whose exponent
is past every double's, so the text is not strict JSON; zk2py passed it
unchanged: 2710 of 2710, `desired` 351.) `readings`
passes 324, 20 more than before:
- nine of zk2py's exact encoding, each read back the same value;
- the encoding's member order;
- a refused non-finite number;
- nine of §2.4's revision read exactly.

**The live half.** Every module is new:
- **`zk2py.desired_live`.**
  - **The author.** It writes a document only when its content
    changes, deterministically, every number by its exact value. It keeps
    a record (the owner example's format). Before its tokens it makes again
    each delete still in its window, for what is left of that window, and
    puts each document again, its clock caught up past every stamp the
    record holds. It can re-mint in its own session.
  - **The target.** It subscribes, then seeds. It follows its author on a
    watch session (core §8.1, 0.37), or reads its presence every 500 ms.
    It seeds at each appearance, and up to four times more when a seed
    draws nothing. It reads an archive only after a complete read shows
    its author absent. `TargetRole` judges each version. The scenarios'
    drive acts (hold, fail-next, refuse-next). It puts its report before
    its tokens and after each change. A re-mint keeps its acts.
- **`zk2py.desired_owner`.** Either party as a process, with the owner
  example's `--desired-*` options and `desired …` commands.
- **`zk2py.desired_scenarios`.** It runs `scenarios.md` §1 to §8 and §10
  with any two parties as black boxes, and two sections of zk2py's own:
  - nine edge documents through each target;
  - a target's re-mint.

`worker.v1` and `tasking.v1`, transcribed from `scenarios.md` alone,
give the owner example's descriptors' fingerprints, `sha256:66e46fce…` and
`sha256:a61f80ba…`. F-177's answer holds.

**`python -m zk2py.desired_scenarios`: every check passes in all eight
runs**, about 60 s each: four pairs of parties, each with targets on a
watch session and polling. The checks per run:
- §1 to §8 and §10: 41 with a zk2py author, 40 with the owner example's,
  whose queryable's GETs (§8) its stdin does not show;
- edges: 9;
- re-mint: 2.

| Author → target | Watch | Polling |
|---|---|---|
| zk2py → zk2py | 52 of 52 | 52 of 52 |
| Rust → Rust | 51 of 51 | 51 of 51 |
| zk2py → Rust | 52 of 52 | 52 of 52 |
| Rust → zk2py | 51 of 51 | 51 of 51 |

§1's target converges 1.11 s after the heal in every pair (spike S12: 1.4
to 1.6 s).

**The two targets agree on every edge document.** A missing required
member is `invalid` at `""`; `1.5` is `invalid` at `/revision`, with no
revision observed. `fly` is at `/mode`; `0` and `10.5` are at `/rate_hz`.
`[1, 2]` is at `""`. `3.0` and `30e-1` are revision 3. 2^64 converges
with no revision observed. zk2py's first run found one bug of its own,
fixed: its pointer was `""` for every refusal, since it did not follow
the type's `$ref`.

**What the owner example could not do from stdin:**
- show its author's queryable's GETs, so §8's "the author's queryable
  receives none" stands on the target's `seeds=0`;
- show what a target's seed found, a read not made included (§2.5: "a
  target says so where it says what its seed found"; the runtime exposes
  it, the example does not);
- put a document by its exact values (F-194).

Everything else ran: hold and release, fail-next, refuse-next, reread,
new-epoch for the author and the target, the record and the window, the
archive, unbound, watch, and tokenless.

**zenoh-python** builds a timestamp's NTP64 from seconds and nanoseconds.
1,516 of 2,000 random raw values cannot be built so (F-196), and §2's X
reuses the timestamp S received.

**Round 53's and round 54's findings at 0.3**, each with a status line
under its entry:
- F-181: resolved, against zk2py's reading;
- F-182: resolved by `config.v1` 0.5;
- F-183 and F-184: resolved, F-184 as zk2py read it;
- F-187: its `desired.v1` half resolved, its `alarms.v1` half open;
- F-191: its `desired.v1` half resolved.

### F-192 · contradiction · desired.v1 0.3 §2.7 "A re-mint is not a restart" against "in an instance"

> §2.7 (0.3): "**A re-mint is not a restart** (0.3). A target that mints a
> new instance id in its own process (core §1.5, §8.1) still runs what it
> applied, and keeps what it acted on and its report: its acts are its
> process's, which a restart loses and a re-mint does not."

> §2.7: "**A restart acts again.** A new instance of the target holds
> nothing of its previous instance's acts, and MUST act on the first
> version it reads"

> §2.7: "**Once per version, in an instance.** A target MUST act on each
> version it admits and does not adopt (§2.6) once"

The text's unit of memory is still "an instance", in a dozen places:
- §2.6, "in this instance", and its 16 remembered ids;
- §2.7, "Once per version, in an instance", and "A new instance of the
  target holds nothing";
- §2.16, "a document it acted on in this instance named";
- §5's table, "no act yet in this instance".

An instance is core §1.5's, the id a re-mint changes. 0.3 now keeps the
acts across a re-mint, so read literally, a re-minted target is "a new
instance … [that] MUST act on the first version it reads" and keeps what
it acted on, both at once. Both implementations keep the acts:
zk2py's re-mint section passes against the Rust target too. The word
means the process now, and the text does not say so.

**zk2py:** the process, as 0.3's sentence says. A target's re-mint keeps
its `TargetRole`.

**Wording that would remove the guess:** §0's terms, "**In an instance**
… of a target: in one process (a re-mint keeps it, §2.7)", or each "in
an instance" read "in one process".

### F-193 · gap · desired.v1 0.3 §2.7 "invalid": the pointer of a refusal no member carries

> §2.7 (0.3): "A document that decodes and that its JSON Schema (core §7.3)
> refuses is `invalid` too, never `undecodable` (0.3): it decodes; its
> `pointer` is where the schema refuses it."

A member's own schema refuses at that member, `/mode`. The text does not
say where a missing required member is refused, nor a document that is no
object. JSON Schema's own output puts both at the object, `""`. zk2py and
the owner example both write `""` (the edges section), but no fixture
pins any pointer of a schema's refusal, and a third implementation might
write `/revision` for a missing `revision`.

**zk2py:** `""` for both, a member's own pointer otherwise
(`schema_pointer`).

**Wording that would remove the guess:** §2.7, "… where the schema refuses
it: a member's pointer for its own schema's refusal, the object's (`""`
at the top) for a missing required member or a value of another type",
with an `order.json` case, or a note that `pointer` is informative here.

### F-194 · contradiction · the owner example's help (#840, DC): `desired put` "as written"

> `owner --help`: "desired put <target> <json>     author: <target>
> (<system>/<service>)'s document, the rest of the line as written"

The example does not write the line as written. It re-encodes it, members
sorted and compact, which §2.11's determinism allows. It also reads its
numbers as doubles:
- `0.10000000000000000555` goes on D as `0.1`;
- `18446744073709551616` goes on D as `1.8446744073709552e+19`.

§2.7 (0.2) compares numbers by their exact values. These are other
values, so its author cannot put two documents that differ past a
double's precision, which is F-166's case and `order.json` case 76. A
runner that drives it to test exact equality tests something else, and
cannot tell. zk2py's author keeps every number's exact value (the edges
section's notes).

**zk2py:** nothing to implement. Its runner notes what each author wrote.

**Wording that would remove the guess:** the help, "re-encoded, members
sorted, numbers as doubles", or the example reading the line with its
numbers exact.

### F-195 · contradiction · desired.v1 0.3 scenarios.md §7 and §10: "(core §8.1, 0.3)"

> scenarios §7 Expected 1: "a complete presence read shows no instance
> token of the author's, which is absent (core §8.1, 0.3)"

> scenarios §10 Expected 1: "a complete presence read through R2 shows
> none, absence by core §8.1 (0.3)"

Placed after "core §8.1", the marker reads as core's version 0.3. The rule
is core 0.37's, and the marker is this text's 0.3, the same slip as
`config.v1`'s F-182.

**zk2py:** nothing to implement.

**Wording that would remove the guess:** "(core §8.1, 0.37; this text
0.3)".

### F-196 · gap · desired.v1 scenarios.md §2: "the timestamp s2, exactly" from a second implementation's binding

> scenarios §2 Setup: "A test session X, a client of R1, can put on `D`
> with a timestamp it sets."

> scenarios §2 step 2: "X puts v3's content on `D` with the timestamp
> `s2`, exactly."

zenoh-python 1.10.1 builds a timestamp from an `NTP64(seconds,
nanoseconds)`. A raw NTP64 time counts 2^−32 s units, finer than a
nanosecond, so most raw values cannot be built: 1,516 of 2,000 random ones
here. Built from `s2`'s text, as a report or a log writes it, X's stamp is
not `s2`. Its time then differs by a few units, so the stamp is admitted,
and acted on, the opposite of step 2's expectation. A Python runner can
only reuse the timestamp object a subscriber received with v2, which
zk2py's does. Step 1's "less 1 s" has no such problem.

**zk2py:** X puts with the `zenoh.Timestamp` S received with s2.

**Wording that would remove the guess:** §2's setup, "X puts with the
timestamp a subscriber received with `s2` (a binding may not build every
NTP64 from its text)".

### F-197 · gap · desired.v1 0.3 scenarios.md §7 and §10: an archive read only on absence is not observed

> §2.5 (0.3): "**The reference** reads an archive only after a complete
> read shows its author absent, never on a silent GET while its author is
> present: an archive can hold a document its author deleted past its
> tombstone window (§2.8), which such a target would act on again."

> §2.9 (0.3): "never on a withdrawal heard live, nor through a session cut
> off (§2.5, 0.3, F-187)."

In §7 the author is gone, and in §10 the target restarts during a cut and
never sees it. In both, the author is absent and its GET silent at once,
so a target that reads its archive on any silent GET, or on a withdrawal
heard live, passes as one that waits for a complete read. No section has a
silent GET while the author is present, a withdrawal heard live with the
author back at once, or a target cut off from every node. §2.5's "a
target says so where it says what its seed found" is observed by no
step, and the owner example shows no seed's finding.

**zk2py:** an archive read only when a seed draws nothing and a complete
read shows the author absent. Its runner checks that read on the bus, not
the target's own.

**Wording that would remove the guess:** a step with the author present
and its queryable silent (held), the archive holding an older version, in
which the target reads no archive and acts on nothing.

### F-198 · ambiguity · desired.v1 0.3 §2.11 "Pacing" against core S2: a kept document not yet put again

> §2.11 (0.3): "A paced author makes its deletes again first, then puts its
> documents at a rate, the tail after its instance token: the SHOULD above
> is given up for that tail, never the MUST, since a GET is answered only
> with what the new instance has put."

> core S2: "It MUST answer a selector over them with every matching live
> key, plus a `reply_del` for each matching key it deleted within the
> window."

A paced author keeps documents it has not yet put again, and answers a GET
without them. Core S2 asks for "every matching live key". The text holds
the two together only if a document kept and not yet put again is not
live, which neither text says. A tool's §2.17 GET in the meantime reads
`author_silent` for that target, and its S1 check sees no stamp. zk2py's
author does not pace, so the case is not run.

**zk2py:** no pacing: every document is put again before the tokens.

**Wording that would remove the guess:** §2.11, "a document is live, for
core S2, from its put by the new instance", or core S2's "live" defined
as put by the instance that answers.

**Counts at desired.v1 0.3, config.v1 0.5 and core 0.37:** 198 entries. Open:
- F-185, F-186, F-188, F-189 and F-190;
- F-187's and F-191's halves outside `desired.v1`;
- F-192 to F-198.

Every other entry is resolved.

## At redundancy.v1 0.1 and core 0.39 (#609)

Round 56 read cold `redundancy.v1`'s first text, its six fixtures (160
cases) and its scenarios, with core 0.39 (the standby mark, D012,
`election` published), core 0.38 (round 54's answers) and `hostid.v1`
0.6 (§2.12's recognised standby). zk2py implemented each from the text.
With the Rust runtime (RB) not yet written, it ran the profile live
against itself.

**Core 0.38.** Each of F-185 to F-191 is resolved; each entry has its
status line. `list_presence` now checks the session's **links**
(`Session.info.links()`, which zenoh-python 1.10.1 has) before and after
a read. A read whose links are gone at its end is possibly incomplete, cut
off during it (`CUT_OFF_DURING`). `presence_scenarios` runs the new steps:
- §4: three pairings;
- §7: 8 checks, step 5 in 10 of 10;
- §8: 11 checks, its five steps through the cut link;
- §9: 9 checks; step 4 counts each session's liveliness GETs; step 5's
  stalled watch link passes for zk2py's follower in 10 of 10, and fails
  for the control in 10 of 10, as predicted.

31 checks, all pass. The bounded GET beside an undrained bounded
subscriber is not run, since it hangs.

**Core 0.39.** D012 is cascade 7, each entry read for `standby` and
`token` alone, and `descriptors` passes the five new fixtures. W105 reads
`redundancy` from the published table (`contracts/w105-redundancy-epoch`).
zk2py's fault shapes read the mark. A standby is no "no token" shape. A
holder marked standby is a new shape, `standby holding`. A "no token" is
read only from a complete read. The owner (`zk2py.owner`) gains a
standby:
- an interface marked at start, validated, with nothing declared;
- `take_over`, core §8.2's steps 1 to 4 for it;
- `step_down`, their reverse;
- a refusal of a write on a standby's keys;
- a refusal at start of an invalid declaration, a governed interface in
  the tokenless set among them, and of a standby on a tokenless one
  (D012).

**`redundancy.v1` 0.1, static.** `zk2py.redundancy` holds the
declaration, an entry's mark, one read's election, the heal, the state
machine and §5's four questions. **160 of 160** passed on their first
run.

**`hostid.v1` 0.6.** `hostid_collision` recognises a standby by
`redundancy.twin` over the finding's two reads, and does not count it.
Those reads are now of `@zk/**` (F-208). `hostid_scenarios` §6 step 6
passes: no finding, and B is said to stand by.

**`just py-conformance`: 2890 of 2890.** `redundancy` is 160 of 160 and
`descriptors` 47. `contracts` is 105 and `layout` 22. `readings` is 336,
twelve more than before:
- three of 0.38's guards, the links read, never the transports;
- nine of the machine's dropped events (F-199) and zk2py's own re-mint
  (F-209), each with what zk2py's runtime gives back.

**The live half** (`zk2py.redundancy_live`, `redundancy_owner`,
`redundancy_scenarios`):
- **The candidate** is an owner standing by, driven through the machine
  by one worker thread, on three sessions:
  - the claim's;
  - a watch, following `zk2/<address>/@zk/**` with history, with its
    round trip;
  - a reader, after one round trip on the claim's session (F-204).
- **Under `external`,** the lock is a `flock`.
- **The consumer** (`Follower`) follows the lowest holder's `meta.zid`.
- **The tool** (`tool_reading`) answers §5 from two reads a grace apart.
- **The runner** drives each candidate as a process. A crash is a
  SIGKILL, and a host lost is a SIGKILL behind `SilentLink`, a relay
  that goes silent and keeps the router's side open.

**`python -m zk2py.redundancy_scenarios`: 59 of 59**, in sections:

| §1 | §2 | §3 | §4 | §5 | §6 | §7 | §8 | §9 | §10 | §11 | §12 | re-mint |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 9 | 3 | 1 (20 runs) | 2 | 4 | 4 | 9 | 3 | 4 | 6 | 3 | 8 | 3 |

**The scenarios, read for sufficiency.** Every section ran, but for
these:
- §12 step 6 ran as `hostid_scenarios` §6 step 6;
- §12 step 7's doctor is zenctl's, so zk2py's own shapes and its §5 split
  stood in;
- §9's held heal is zk2py's hook (`--hold-heal`).

Four sections ran only after a change of what they observe:
- §1 and §10 step 3 run both ways, as written and on a silent link
  (F-201);
- §1 and §3 observe the order by a GET at each token (F-202);
- §3 judges by §2.5 step 2 (F-203);
- §10 step 4 is judged by §2.11 (F-200).

§5 holds only with the reads on a session apart from the claim's
(F-204), the peers listening (F-206), and the reconnect held at 0.5 s
(F-205).

**What zenoh-python could not do:** nothing the scenarios need, but the
bounded GET of §4. Two facts are zenoh's, not the binding's:
- the order of a put and a token at a subscriber (F-202);
- a peer's reads through a router (F-206).

### F-199 · gap · redundancy.v1 0.1 `lifecycle.json`, §2.5, §2.8, §2.9: what is heard while a read runs

> `lifecycle.json`: "Every other event in every other state is nothing,
> and changes nothing."

> §2.9: "a read not made shows none (core §8.1, 0.37, 0.38)"

A read takes a round trip, and the watch keeps hearing during it. The
machine drops three things a bus can hand it:
- **An appearance while it campaigns.** B's read misses A's claim, and
  A's token reaches B's watch while that read is in flight. B takes over.
  A, the lower, hears B appear, heals and stays. B never reads for a heal,
  so the split lasts. Only an appearance while taking over is kept.
- **A withdrawal while it campaigns.** The read shows an incumbent that
  went while it ran. The machine stands by on it, no retry armed, and
  waits for a withdrawal it already dropped.
- **A heal's read not made.** It shows none, so holders are own alone,
  and the machine goes back to serving. The appearance is gone, and the
  split is let go. A watch session that reconnects before the reading
  session does this after a partition.

zk2py's runs never hit the first two, but §3 shows appearances heard
while campaigning, and the windows are a round trip wide.

**zk2py:** the fixture's machine, and `redundancy.given_back`. Its runtime
hands back what its watch holds:
- after a takeover, an `appeared` per other holder heard;
- after a stand-by on an incumbent, a `withdrawn` when none is heard;
- after a heal's read not complete, an `appeared` again after the retry
  interval.

Seven readings show the drops and the remedy.

**Wording that would remove the guess:** `lifecycle.json`, "an appearance
or a withdrawal heard while a read is asked is kept, and decided after
that read's decision", and §2.9, "a heal whose read is not complete reads
again after the retry interval".

### F-200 · contradiction · redundancy.v1 scenarios.md §10 Expected 4 against §2.11 and `elections.json`

> scenarios.md §10 step 4: "V1 and V2 start with two lock files (a
> misconfiguration), V2 first; the test waits 10 s." Expected 4: "Both
> serve: two interface tokens, and neither steps down."

> §2.11: "**With the lock,** it takes over (§2.6) once a read shows no
> other holder of the interface's token"

> `elections.json`: "external: another instance holds the token, which
> may outlive the lock" → stand_by, incumbent

V2 starts first, holds its lock and serves. V1 then holds its own lock,
and reads V2's token, so §2.11 stands it by. Both serve only if V1's read
misses V2's token, as in a simultaneous start. zk2py measured V1 standing
by on V2 in every run.

**zk2py:** judges step 4 by §2.11.

**Wording that would remove the guess:** step 4, "V1 and V2 start within
10 ms of each other", or Expected 4, "V2 serves; V1, its own lock held,
stands by on V2's token: a lock that two hosts both hold is a split only
when their reads miss each other".

### F-201 · contradiction · redundancy.v1 §2.8 "When a departure is seen", scenarios.md §1 step 4 and §10 step 3, against core Appendix B

> §2.8: "A crash is seen at zenoh's lease, 10 s by default, when the
> incumbent's router withdraws its tokens (core Appendix B)."

> scenarios.md §1: "4. The test kills A's process (SIGKILL), at instant
> K." Expected 4: "S hears A's tokens withdrawn when R1's lease for A's
> session runs out, 10 s after K by default"

> core Appendix B: "The withdrawal reaches the reader within 1–2 ms when
> the link closes, and one lease after a link goes silent"

A process killed on a live host has its TCP links closed by its kernel.
So the router withdraws its tokens at once: zk2py measured 2 ms after
SIGKILL, in §1 and in §10 step 3. The lease shows only when the link
goes silent: the host lost, or a partition. Behind a relay that went
silent before the kill, zk2py measured 9.96 s. §1's "B made no read
between K and the withdrawal, and never polled" is then tested at all.
§10 step 3's "waits for V1's interface token to go (V1's lease)" depends
on the same lease.

**zk2py:** runs §1 and §10 step 3 both ways. As written: withdrawn at
once, B (or V2) serving 0.1 s later. On a silent link: withdrawn at the
lease, B serving within 3 s, no read before it. Every check passes.

**Wording that would remove the guess:** §2.8, "a crash whose host's
kernel closes its links is seen at once; a lost host or a partition at
zenoh's lease"; and §1 and §10, "the test makes A's link silent, then
kills its process".

### F-202 · gap · redundancy.v1 scenarios.md §1 Expected 1 and §3 Expected 1: an order no subscriber sees

> §1 Expected 1: "S receives A's first descriptor, marked standby, then
> its instance token …, and later, once A takes over, its descriptor
> without the mark, then its interface token"

> §3 Expected 1: "Each instance's descriptor, marked standby, reaches S
> before its instance token"

zenoh carries a token's declaration and a put on different priorities.
One session put a key, then declared a token, a hundred times. A
subscriber session on the same router received the token first in 72 of
100. An owner that keeps §8.2's order fails these Expected about as often
as one that does not, and S's two kinds of delivery cannot be ordered.
Core `presence.md` §1 observes the same order the robust way: "A GET of
the descriptor … made the moment the instance token appears … answers".

**zk2py:** S gets the descriptor the moment each token appears. At the
instance token it must read marked, unless the interface token beat that
GET. At the interface token it must read unmarked. S's arrival order is
noted, never judged.

**Wording that would remove the guess:** "a GET of A's descriptor, made
the moment S hears its instance token, answers it marked standby, and
one made the moment S hears its interface token, unmarked (core
presence.md §1's observation)".

### F-203 · contradiction · redundancy.v1 scenarios.md §3 Expected 1 against §2.5 step 2

> §3 Expected 1: "In every run, A serves and B stands by within 5 s of
> the later start plus two graces. … In a run where both read before
> either claim reached the other, both take over; then B, the higher,
> steps down"

> §2.5 step 2: "A live incumbent never yields to a lower id: the lowest
> id is a tie-break for simultaneous starts, not an entitlement."

There is a third order. B's read misses A's claim, and B takes over
before A reads. A's read then shows B's token, so A stands by on a live
incumbent. One holder means no heal. zk2py saw it in 2 of 20 runs in each
of two batches, and in none of 20 in each of two others.

**zk2py:** each run must end, within the bound, with A serving, or with
B serving and A standing by on B's token, which §2.5 step 2 asks.

**Wording that would remove the guess:** "one serves and the other stands
by: A, unless B took over before A's read, A then standing by on B as
§2.5 step 2 says".

### F-204 · gap · redundancy.v1 §2.5 step 1 and step 3: a refused read on the claim's own session

> §2.5 step 1: "**Its read follows its claim.** The read goes on a
> session that has never held a liveliness subscriber … When that is not
> the session holding the claim, the instance makes one round trip on
> the claim's session"

> §2.5 step 3: "it shows the instance's own claim. A read that misses the
> instance token the instance knows it holds missed what it could not
> see, its claim not yet at its router, or the read refused (core §8.1's
> refused read is complete and empty)"

The text allows the read on the claim's own session. zenoh answers a
session's liveliness GET with the session's own tokens locally. Through a
router whose access control denies `liveliness_query` on ingress, zk2py
measured two outcomes:
- on the claim's session, the read is complete and holds the claim;
- on another session, it is complete and empty.

So step 3's check, the only one that tells a refused read from a lone
candidate's, never sees a refusal on the claim's session. §5 step 3
failed that way, A serving through R3, until zk2py moved its reads.

**zk2py:** reads on a session of its own, neither the claim's nor the
watch's, after one round trip on the claim's session. §5 step 3 passes:
five reads, each complete, empty and missing the claim.

**Wording that would remove the guess:** step 1, "The read goes on a
session that holds neither a liveliness subscriber nor the claim, since
a session answers its own tokens itself; the instance makes one round
trip on the claim's session …".

### F-205 · minor · redundancy.v1 scenarios.md §5 Expected 2: zenoh's reconnect inside the bound

> §5 Expected 2: "Within the retry interval plus 2 s of R1's start, A's
> next campaign reads complete, finds its own claim, and A serves."

A, a peer, opened 4.5 s before R1. zenoh retries a peer's connect at a
period that grows to 4 s by default, and A served 4.5 s after R1's
start. With the period held at 0.5 s, A served 0.5 s after. The bound
assumes a reconnect the text does not configure.

**zk2py:** its peers retry every 0.5 s.

**Wording that would remove the guess:** the Setup, "A's connect retry
period at most 1 s", or "within the retry interval plus its reconnect".

### F-206 · gap · core §8.1 "complete", and redundancy.v1 §0 "Read": a peer's read through a router

> redundancy.v1 §0: "**Complete** is core §8.1's: it ran to the routers'
> final reply with no error reply, on a session that has never held a
> liveliness subscriber …, and that session's links held both before the
> read and when it ended"

> scenarios.md §5 step 1: "A starts as a peer whose only endpoint is R1's"

zenoh routes no peer's tokens to another peer through a router, at its
default peer mode. zk2py ran two peer sessions with no listener behind
one router, one holding a token. The other read the address complete and
empty every time, for 15 s, while a client of the same router read the
token. Every condition of "complete" held. With both peers listening,
they linked through gossip, and the token was read after 0.2 s, or 3.6 s
when the router came up late. A candidate that is a peer, beside an
incumbent that is one, can read a complete vacancy and take over.

**zk2py:** its peers listen on loopback. Its candidates are clients in
every other section.

**Wording that would remove the guess:** §8.1, "a peer's read is complete
only for the tokens its routers or its linked peers hold: a peer that
other peers cannot link to reads none of theirs", and redundancy.v1 §2.13
or §2.5, "candidates are clients of a router, or peers linked to each
other".

### F-207 · ambiguity · redundancy.v1 `lifecycle.json`: a withdrawal kept while stepping down, under external without the lock

> `lifecycle.json`: "finish of a step_down stands by, and campaigns when
> a withdrawal was heard while stepping down, else nothing; while standing
> by, a withdrawal campaigns (under external only with the lock)", and
> "the claim's finish campaigns (a read, or under external without the
> lock stand_by no_lock)"

Under `external` a step-down follows a lost lock, so the instance lacks
it at the finish. Does a withdrawal kept while stepping down campaign
then? "Campaigns" would give `stand_by` with `no_lock`. "Only with the
lock" would give `nothing`. No case asks.

**zk2py:** `nothing`, reading the withdrawal kept while stepping down as
one heard standing by.

**Wording that would remove the guess:** "campaigns when a withdrawal was
heard while stepping down and it may (under external, with the lock),
else nothing", and a case.

### F-208 · gap · hostid.v1 0.6 §2.12: the finding's reads hold no interface token

> hostid.v1 §2.12: "A tool reads the instance tokens of an address
> (`zk2/<system>/<service>/@zk/instance/*`, core §8.1) twice, a grace
> apart"

> §2.12 (0.6): "that profile's question 'is this instance a standby, or a
> twin?' answers **no**, `standby`, of each, from the same two reads"

> redundancy.v1 §2.15: "X2 holding no token, marked standby: **no**,
> `standby`, when the second read is complete"

The fourth question turns on whether the instance holds the interface's
token. A read of `@zk/instance/*` shows none, so "from the same two
reads" cannot answer it.

**zk2py:** the finding reads `zk2/<address>/@zk/**`, the instance tokens
and the interface tokens at once, as `redundancy.v1` §0's read does.

**Wording that would remove the guess:** §2.12, "reads the address's
`@zk/**` (its instance and interface tokens) twice".

### F-209 · gap · redundancy.v1 §2.5 to §2.9 and `lifecycle.json`: an incumbent's own re-mint

> §2.8: "**A re-mint is no vacancy.** An incumbent's make-before-break
> re-mint holds its new token before its old one goes (core §8.1)."

> `lifecycle.json`: "while serving under claim, another instance's
> appearance reads; that read's holders, own counted whether it shows own
> or not"

The text gives the re-mint from a standby's side only. The machine knows
one id, `own`, and has no event for a re-mint:
- the incumbent's watch hears its new id's token, which is "another
  instance's appearance", and it reads;
- the overlap is two holders, so the grace is armed;
- the second read shows the new id, and `own`, the old id, is counted
  beside it. When the new id is lower, it steps down to itself.

A reading shows it.

**zk2py:** `Machine.remint`: the new id becomes `own`, and the former ids
are its own, neither an appearance nor another holder. The candidate's
`new_epoch` tells the machine first. Its `re-mint` section passes: the
new token before the old one goes; no read, no step-down, no takeover by
B; A serving under its new id.

**Wording that would remove the guess:** an event `{"reminted": id}`
whose id is `own` from then on, its former ids never another holder, and
§2.9, "a candidate's own re-mint is no appearance".

**Counts at redundancy.v1 0.1 and core 0.39:** 209 entries. F-199 to
F-209 are open, F-205 minor. F-181 to F-184 are resolved, and F-185 to
F-191 by core 0.38. F-192 to F-198, against desired.v1 0.3 and config.v1
0.5, are open.
