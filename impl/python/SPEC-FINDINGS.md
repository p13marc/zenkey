# Findings against the zk2 spec (from the Python implementation, #609)

zk2py was written from `spec/` alone: `core.md`, the two JSON schemas,
`core/`, the fixtures and the scenarios, plus `examples/zk2/` as extra
inputs. It never read the Rust implementation or `docs/zk2/`, and it runs
the Rust owner example only as a black box. Each entry below is a place
where that was not enough, or where the spec said two things.

**Twenty-two rounds.**
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
  the Rust owner example (see "At health.v1 0.2" at the end).

**Severities.**
- **gap:** the prose is silent. The entry says whether a fixture's expected
  value decided it, or zk2py guessed.
- **ambiguity:** the text allows two readings. A fixture or a guess decided.
- **contradiction:** the prose, read literally, and a fixture disagree, or
  two parts of the spec do.
- **blocker:** zk2py could not implement the rule. None was found.

**Counts at core 0.23, hostid.v1 0.3, freshness.v1 0.2 and health.v1 0.2:**
104 entries, none open.
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

Code comments cite open entries as `SPEC-FINDINGS F-nn`, and resolved ones
by the spec section that now states the rule.

| Id | Severity | Status at 0.23 | Location | In one line |
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
