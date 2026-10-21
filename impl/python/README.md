# zk2py: the second implementation of the zk2 spec

zk2py is an independent Python implementation of the **static half** of the
zk2 core specification, plus the **first slice of its live half** (issue
#609).
The live slice covers presence, descriptors and contract retrieval; see
"The live half" below. It exists to test a claim the spec
makes about itself (`spec/core.md`, preamble): an implementation in any
language can be built from `spec/` alone, without reading the Rust
reference.

It was written from `spec/` (prose, schemas, fixtures, scenarios) and
`examples/zk2/` only. It never read `zenkey-model/`, the other Rust crates
or `docs/zk2/`. Where `spec/` was not enough, the gap is recorded in
[`SPEC-FINDINGS.md`](SPEC-FINDINGS.md). That file is as much the
deliverable as the passing runner.

## Running it

From the repository root:

```bash
just py-conformance
```

The recipe runs `impl/python/bootstrap.py`, then the runner. It:
1. creates `target/py-venv` with `python3 -m venv --without-pip`, which
   works where `ensurepip` is missing (Debian/Ubuntu without
   `python3-venv`);
2. downloads a pinned, sha256-checked `pip` wheel and installs pip into the
   venv by running pip from its own wheel;
3. installs [`requirements.txt`](requirements.txt), with exact versions;
4. uses `protoc` from `PATH` if it reports `libprotoc 3.21.12`. Otherwise it
   downloads the pinned, sha256-checked protoc 21.12 release archive
   (linux x86_64) into the venv;
5. runs `python -m zk2py.conformance spec/conformance`, with
   `ZK2PY_PROTOC` naming that compiler.

The bootstrap uses only the standard library and needs no sudo. It reruns
quickly: the venv, pip and the requirements are reused when they are
unchanged. Python 3.11 or later is required, for `tomllib`.

To run it by hand once the venv exists:

```bash
PYTHONPATH=impl/python ZK2PY_PROTOC=/usr/bin/protoc \
  target/py-venv/bin/python -m zk2py.conformance spec/conformance [--family contracts]
```

The runner prints a pass/fail count per family, then every failure with its
case name. It exits with:
- **0** when everything passes;
- **1** when any case fails;
- **2** when it could not run: a missing directory, a broken fixture file,
  or no usable protoc.

## What it covers

| Family | Spec | Module | What is checked |
|---|---|---|---|
| layout | App. E | | every entry of `spec/conformance/` is read by a family, and every profile with a `conformance/` directory has one: a new one fails until it is |
| keys | §1.1, §1.2 | `keys`, `lexical` | parse every key in `keys.json`, refuse the others, and build each accepted key back to the same string |
| slugs | §1.4 | `slug` | slug and unslug, round trips, and refusal of non-canonical chunks |
| templates | §2.2 | `templates` | match, then rank, with unslugged bindings |
| contracts | §9.1–§9.5, App. D | `contract`, `schemas`, `shape`, `protoc`, `jcs` | the exact sorted codes, the canonical JCS bytes and the fingerprint of every fixture. The protobuf well-known types and the five `e000-toml11-*` cases (0.4) are included. |
| sets | §9.2 | `sets` | E035, E036 |
| bundles | §9.6 | `bundle` | the twelve verification steps and their tags. `seed.toml` also **builds** to `valid.bundle.json` byte for byte. |
| history | §9.7 | `history` | `[at, tag]` per history root |
| descriptors | §3.3 | `descriptor` | the D codes, against `descriptors/contracts/nav.v2.toml`. That contract's fingerprint, computed here, is the `sha256:fea2…` the fixtures expect. |
| errors | §5.2 | `envelope`, `cbor` | JSON, CBOR and protobuf envelopes, and the refusal tags |
| budget | §2.7 (0.24, 0.25, 0.27) | `budget` | every file of `budget/`: `bounds.json` (the bound across instances), `population.json` (a GET's or a window's population against it), `rate.json` (an event's rate per member) and `lossless.json` (whether a window is shown to have lost no delivery, 0.25; its `reader_stall` and `stall_bounds`, 0.27). An unknown file there fails. |
| seed | §4.2 (0.27) | `seed` | `seed/merge.json`: `order` (two stamps, or a stamp and none, compared) and `cases` (a key's arrivals, each kept or not). An unknown file there fails. |
| compat | §9.7, §9.8 | `compat` | **all 100 cases** (spec 0.8), evaluated through `compat/README.md`'s one-resource wrapper: §9.8's six tables, the JSON Schema and protobuf rules, `same_revision`, and the FULL_TRANSITIVE cases (with each pairwise `against`). |
| hostid | profiles/hostid §2.1, §2.11 | `hostid` | every file of `spec/profiles/hostid/conformance/`: `vectors.json` (the derivation) and `shapes.json` (the minted shape). An unknown file there fails (`profiles/README.md`). |
| freshness | profiles/freshness §2.1–§2.8 | `freshness` | every file of `spec/profiles/freshness/conformance/`: `horizons.json` (a resource's horizon), `judgements.json` (one member's combined verdict, seconds read exactly to the nanosecond) and `clock-trust.json` (a measured clock's trust over one reading, 0.2). An unknown file there fails. |
| health | profiles/health §2.2, §2.10, §2.11 | `health` | every file of `spec/profiles/health/conformance/`: `judgements.json` (§2.11's procedure over one reading), `rollups.json` (a tool's roll-up), `codes.json` (fault codes) and `clock.json` (§5's clock question, 0.3). An unknown file there fails. |
| config | profiles/config §2.2–§2.19 | `config` | every file of `spec/profiles/config/conformance/`: `validation.json` (§2.5's, §2.10's and §2.13's orders of refusals), `window.json` (§2.8's window and deadline), `lifecycle.json` (the owner as a state machine: each step's reply, device acts, events and state), `desired.json` (§2.18) and `redaction.json` (§2.15). An unknown file there fails. |
| link | profiles/link §2.1–§2.15 | `link` | every file of `spec/profiles/link/conformance/`: `exposure.json` (§2.2's exposure and whether a resource's pushes cross), `plan.json` (a face policy and a deployment to each face's plan, or its refusal) and `lints.json` (the policy's linter, the plan's warnings, the windows and a router's block). An unknown file there fails. |
| alarms | profiles/alarms §2.2–§2.6, §2.12, §2.14, §2.15, §5 | `alarms` | every file of `spec/profiles/alarms/conformance/`: `keys.json` (the recipe through the sanitizing step, the strict recipe beside it, host-scoped names, references and their keys), `documents.json` (readable, the owner's errors and warnings; a case given as its `text` read as strict JSON first, 0.2), `transitions.json` (one key's samples read in stamp order) and `conclusions.json` (§5's four questions, and §2.15's verdict on an owner's alarms). An unknown file there fails. |
| readings | | `alarms` | not a fixture family: zk2py's own cases, each naming what it reads (an open finding's reading, or a stated rule beyond its fixture). Today, 137: 11 of `alarms.v1` §2.4's whitespace (RFC 8259's `ws` around a payload's value reads; anything else after it, and a byte-order mark before it, is `json`), six of §2.15's window healed only by a GET asked after a break was seen, two of 0.5's whole scope (an archive's read plays no part), 86 of §2.16's change reporter (over every `transitions.json` case, every `conclusions.json` reading, four loss sequences, four of 0.6's first answers, and four of a key gone from the scope), five of 0.6's presence read dated by its ask, six of §2.14 step 5's left-out key, a `reply_del` among them, and 21 of 0.7's two closed lists: each of the twenty neither reasons the first question gives, read from a reading of its own, on its list, and the table's reader's-sight reasons 0.7's nine. |
| examples | §9.6–§9.8, §10 | | Every `examples/zk2/**/<name>.v<major>.toml`, and every profile's standard contract under `spec/profiles/` (0.23): loads with **no finding at all**, W107 included; its built bundle verifies; it is published in its tree's history root (`examples/zk2/.history` or `spec/profiles/.history`), **byte-identical** to the bundle zk2py builds; it is `compatible` with its history. Both roots pass the §9.7 check, and a revision held in both is one file, byte for byte. |

The result at the time of writing:

```text
layout         19 passed     0 failed
keys           55 passed     0 failed
slugs          42 passed     0 failed
templates      11 passed     0 failed
contracts     101 passed     0 failed
sets            4 passed     0 failed
bundles        24 passed     0 failed
history        10 passed     0 failed
descriptors    42 passed     0 failed
errors         45 passed     0 failed
compat        100 passed     0 failed
budget         95 passed     0 failed
seed           20 passed     0 failed
examples      108 passed     0 failed
readings      137 passed     0 failed
hostid         42 passed     0 failed
freshness     102 passed     0 failed
health        112 passed     0 failed
config        330 passed     0 failed
link          172 passed     0 failed
alarms        316 passed     0 failed
total        1887 passed     0 failed
```

The figures are against `core.md` 0.34, `hostid.v1` 0.5, `freshness.v1`
0.3, `health.v1` 0.4, `config.v1` 0.3, `link.v1` 0.7 and `alarms.v1` 0.7.
`alarms.v1` 0.7 changed no fixture, and 0.6 only `conclusions.json`'s
description.
`alarms.v1` 0.5 gave every `conclusions.json` reading `left_out` and every
`any_firing` case `scope_whole`, and added twelve cases (107), and 0.4 none.
`alarms.v1` 0.3 added four `documents.json` cases, given as their `text`
(RFC 8259's whitespace), which zk2py passed unchanged. `alarms.v1` 0.2
added 19 cases, nine of `documents.json` given as their
`text` (strict JSON first), three of `transitions.json` and seven of
`conclusions.json`, and changed none. `alarms.v1` 0.1 added its four
fixture files, 281 checks, and moved its
standard contract under `spec/profiles/`: one more `examples` check, its
revision in both history roots. `layout` now checks the six profiles'
directories that hold fixtures. Core 0.34 added
`contracts/w105-alarms-vocabulary`, and 0.33 changed no fixture. Three
`lints.json` cases came after `link.v1` 0.7, with no text change, and
zk2py passed them unchanged. `link.v1` 0.7
added 13 `lints.json` cases and changed three; 0.6, core 0.31 and core
0.32 changed no fixture. `link.v1` 0.5 added ten `lints.json` cases
and changed one. `link.v1` 0.4
added 18 `lints.json` cases, and 0.3 one `plan.json` case and eight
`lints.json` cases. `link.v1` 0.2
added the scenarios' radio as an example (`examples/zk2/link/radio.v1.toml`,
4 more `examples` checks). `link.v1` 0.1 added its three fixture files, core 0.29 `contracts/w105-link-downsample`,
and `config.v1` 0.3 seven `validation.json` cases and one `lifecycle.json`
case. `config.v1` 0.1 added its five
fixture files and its standard contract (4 more `examples` checks), and
`layout` checks the four profiles' directories. Core 0.28 added three
`errors` cases, and `config.v1` 0.2 eight `validation.json` cases and two
`lifecycle.json` cases. 0.22 added `contracts/e002-two-majors`, 0.24
the `budget/` family, 0.25 its `lossless.json`, which 0.27 reshaped, and
0.27 the `seed/` family. `freshness.v1` 0.2 added `clock-trust.json` and four
judgement cases. `health.v1` 0.1 added three fixture files, and 0.3
`clock.json`. Five earlier releases added descriptor
fixtures:
- 0.10, `descriptors/ok-optional-role`;
- 0.11, `ok-optional-unchecked`;
- 0.17, `d011-tokenless-archive` and `ok-archive`;
- 0.18, `d011-bad-fingerprint`;
- 0.19, `ok-derivation-profile`.

- Amendments 0.5 to 0.18 resolved F-01 to F-93. `hostid.v1` 0.2 resolved
  F-94 to F-97, core 0.22 F-98, `hostid.v1` 0.3 F-99, `freshness.v1` 0.2
  F-100 to F-102, `health.v1` 0.2 F-103 and F-104, core 0.25 F-105, and
  core 0.27 F-106.
- `config.v1` 0.2 resolved F-107 to F-113, found against its 0.1, and
  overturned one of zk2py's guesses (F-107's values).
- `config.v1` 0.3 resolved F-114 and F-115, found against its 0.2, both as
  zk2py guessed.
- `link.v1` 0.2 and core 0.30 resolved F-116 and F-117, found against
  `link.v1` 0.1, both as zk2py measured.
- `link.v1` 0.3 resolved F-118 and F-119, found against its 0.2. F-119
  went as zk2py guessed. F-118 kept zk2py's guess and added what it
  lacked: every other principal's subject narrowed off the face, and under
  `allow` the face's own subject.
- `link.v1` 0.4 resolved F-120, found against its 0.3: §2.15 now reports
  its three blocks, and two more of the same kind.
- `link.v1` 0.5 resolved F-121, found against its 0.4, as zk2py read it:
  one kind of name per subject.
- Core 0.31 and `link.v1` 0.6 resolved F-122, found against `link.v1`
  0.5's scenario §11. A generator MAY leave a principal's own keys out of
  its complement, and zk2py's, which denies them per message and flow,
  conforms. §11's expected 7 gives that branch's outcome exactly.
- Core 0.32 resolved F-123, found against its 0.31, in zk2py's wording:
  a principal's complement is taken per message and flow.
- `link.v1` 0.7 resolved F-124 and F-125, found in a last cold read of its
  0.6: `block_disabled`, and a far principal of two faces carrying both
  plans' denies, warned by `far_of_two_faces`.
- A cold read of `link.v1` 0.7 found nothing more to guess
  (`SPEC-FINDINGS.md`, "At link.v1 0.7").
- `alarms.v1` 0.2 resolved F-126 to F-132, found against its 0.1. F-127
  went against zk2py's reading (`labels` absent equals `{}`), and F-130
  changed what a window across a face asks. The others went as zk2py read
  them.
- `alarms.v1` 0.3 resolved F-133 (the owner example's `--help` now
  documents its `--alarms` surface whole) and F-134 (§2.4's whitespace is
  RFC 8259's `ws`), both as zk2py read them.
- `alarms.v1` 0.5 resolved F-135 (§5's SHOULD observed by a GET at the
  token), F-136 (the establishing GET heals step 4 for a key it leaves
  out) and F-137 (`scope_incomplete`; an archive's read plays no part,
  against zk2py's reading).
- `alarms.v1` 0.6 resolved F-138 (`conclusions.json`'s description now
  states `left_out` and `scope_whole`) and F-139 (each key keeps its last
  report through a loss), both as zk2py read them.
- `alarms.v1` 0.7 resolved F-140: a first neither splits into two closed
  lists.
- F-141 (freshness.v1's `silent`, the reader's own sight by the list's
  words, is on neither named list and so falls to "any other reason") is
  open (`SPEC-FINDINGS.md`, "At alarms.v1 0.7"). zk2py, which reports no
  first neither, is unaffected.
- They decided 13, 3, 1 and 2 of zk2py's guesses the other way. Since
  then, `hostid.v1` 0.2 decided two more the other way (F-96, F-97), 0.22
  one (F-98), `freshness.v1` 0.2 two (F-101, F-102), `health.v1` 0.2 one
  (F-104), and core 0.27 one, the order of ids at one time (F-106).
- 0.7 adds the nullable reading (C-1) and `$ref`s followed inside
  `oneOf`/`anyOf`/`prefixItems` (X-1) to the classifier.
- 0.8 makes `["null"]` a null schema too, and compares a recursive `$ref`
  inside those keywords by its text, not its target.

zk2py now follows the stated rules; `SPEC-FINDINGS.md` says, per finding,
how.

**The classifier since 0.3.** Against 0.2, zk2py classed every change §9.8
did not list as review. 0.3 lists them, so zk2py now follows the tables,
and its reasons carry the reference rule names (`explicit_set`,
`reserved_reused`, …). Twelve earlier guesses changed:

| Change | 0.2 guess | 0.3 rule |
|---|---|---|
| required → optional | review | breaking |
| a required resource added | review | breaking |
| a template parameter's type changed | review | breaking |
| the payload or attachment encoding changed | review | breaking |
| best_effort → reliable | review | compatible |
| `fanout` forbidden → allowed | review | compatible |
| `replies` many → one | review | compatible |
| a role removed | review | compatible |
| a role required → optional | review | compatible |
| `deprecated` added | review | compatible |
| a raw `media_param` changed | breaking | review |
| `items` toggled between absent, `true` and `false` | breaking | compatible |

Resources are still paired by template, not "by kind token and template",
because the explicit rows need it (F-40).

## The live half (first slice)

```bash
just py-live
```

The recipe reuses the bootstrap, builds the Rust owner example
(`cargo build -q -p zenkey --example owner`, a no-op once built), then
runs `python -m zk2py.live_interop`. The owner is used as a black box,
through its documented contract only.

The runner makes two owner runs:
- `nav.v2`;
- `camera.v1`, `zs.snmp.v1` and zk2py's own `interop/zk2py_probe.v1.toml`.

In each run, zk2py connects as a client and acts on presence, not on the
owner's `ready` line. It checks:
- **Presence (§8.1, §8.2):** one instance token, and one interface token per
  interface. Each token's `<fp16>` comes from the fingerprint zk2py computes
  from the TOML.
- **The descriptor (§3.3):** one reply on the instance key, no D code
  against the contracts, the expected interfaces with zk2py's fingerprints,
  minor, token and capabilities, and every contract-declared role (R3).
- **Retrieval (§8.4):** every bundle, verified against the descriptor's
  fingerprint and byte-identical to zk2py's own build.
- **Retrieval failures:** an unheld revision is unavailable after
  `BestMatching` then `All`. A corrupt nearest holder (a queryable zk2py
  declares) is refused while the valid bundle is accepted, and only corrupt
  holders leave the contract unavailable.
- **Presence at scale (presence.md §4):** 2,000 extra tokens, listed in
  full while a liveliness subscriber is held.
- **Shutdown:** closing the owner's stdin makes it exit 0.

Since 0.5, the runner also checks what §3.2, §3.3, §8.1 and §8.4 now
state:
- the descriptor reply carries no timestamp and no attachment;
- `profiles` is the union of the contracts' `uses` (with `hostid.v1` for
  a minted system, 0.19), in §9.5's order (0.20);
- unbound roles are listed with `bindings: []` and `params: {}`;
- no member token while the owner publishes nothing;
- each holder answers one `application/json` reply;
- a third owner run, on `thruster.v1`, whose required role is unbound,
  must not start.

Every wait is the 1 s that §8.1 gives a conformance run.

**State, operations, and being an owner (0.6).**
- `zk2py.live.get_state` makes a state GET per S4: target `All` and
  consolidation `Latest`. It returns current replies with their stamps, or
  silence.
- `zk2py.live.call` makes a call: `BestMatching` (O1), or `All` for a
  fan-out (O2), with consolidation `None`. It returns a value, a decoded
  §5.2 envelope, a transport error, or silence (O5).
- `zk2py.owner.Owner` is a minimal owner, a router or a client of one. It
  brings itself up in §8.2's order: a stamped raw state value (S1, S2),
  parameterless operations answered per O1–O3 (echo, a JSON operation,
  `app` refusals), the descriptor and bundle holders, then its tokens. It
  refuses to start with an unbound required role (§3.2).

**Since 0.7.** The owner exposes what 0.7's §8.2 calls exposed. Step 2
runs first, so a refusal declares nothing; it refuses what the rule
refuses. Every resource it is not told to withhold is exposed:
- a state by its state queryables and, when parameterless, a publisher. A
  raw state holds `ok` from the start, put before the tokens; any other
  holds no value;
- a stream by its publisher;
- an event, and any template with no member, by nothing more;
- an operation by a `complete` queryable on its key or over its template.
  Over a template, a well-formed member is `not_found` and a key naming no
  member `invalid_request`.

A capability-gated resource whose capability it lacks is implied absent,
and its operation answers `unavailable`. The first descriptor is put in
step 3, before the bundles' queryables.

Three more zk2py-owner runs follow 0.7's scenarios through a router R1 of
the runner's own:
- state.md §1: S1 against an unstamped control put, and the tick;
- presence.md §1: a call, a state GET, the descriptor and the bundle at the
  moment the tokens appear, the first descriptor seen by a data subscriber,
  and a template with no member;
- presence.md §2 steps 4 and 5, each with its control.

**Since 0.8.**
- **Presence completeness (§8.1).** A liveliness GET is complete when the
  routers' final reply ends it with no error reply. zenoh 1.10.1 ends one
  that reached its timeout with the error reply `Timeout`. `Presence.reading`
  states a read as a tool should: possibly incomplete, or complete for
  this reader. A complete, empty read is "absent, or refused by access
  control", since a refused read looks exactly like absence.
- **Retrieval (§8.4).** A token's `fp16` leads to the full fingerprint
  through the instance's descriptor (`live.fingerprint_of`), and every
  retrieval uses that fingerprint.
- **Operations over a template (§5.1).** The owner takes an application
  handler per operation (`handlers`, called with an `OpCall`), or serves
  listed members with a queryable each (`members`).
  - Before any handler runs, it refuses `fanout_forbidden` (O2), then
    `invalid_request` for a concrete parameter chunk that is not a
    canonical slug, fan-out or not.
  - A handler names one member per call, whatever `replies` is. A second
    member, or one the call does not select, is refused to it.
  - A call left without its answer is `internal`.
  - `live.call(first=True)` returns at the first value or envelope.
  - `hold_s` keeps each query open after its reply.
- **The tick (state.md §1).** The scenario now asks only that v4's stamp
  exceed v3's. The runner checks the tick with `Owner.clock`, a clock zk2py
  controls.

Four more runs:
- **The owner example behind R1**, with its new `--connect` (F-69's setup).
  Its value held from the start is stamped with its own zid, not R1's.
- **presence.md §6**, on a zenoh-python router with the scenario's access
  control:
  - a refused alive read is complete and empty, with no error reply;
  - a read through a link the runner stalls (`_StallProxy`) ends with
    `Timeout` and is reported possibly incomplete;
  - the same deny on `egress` alone refuses nothing.
- **operations.md §1**, by behaviour:
  - a call returns on its first reply while the query stays open, and
    under `Latest` it waits;
  - two instances on one router run 200 calls between them;
  - two instances across two routers run 200 each.
- **operations.md §2** steps 1 to 6, and §3 step 3. Three `tc` hosts (one
  queryable per member, one over the template, one refusing `busy`) and
  two `many` scans, all zk2py owners. The second scan's handler names no
  member and sends nothing.

**Since 0.9.**
- **The order of refusals (§5.1):** `fanout_forbidden`, then
  `unavailable`, then a key that names no member. The owner's
  `unavailable` queryable now checks O2 first. A check on `calibrate`, an
  optional operation no host exposes, pins the whole order.
- **presence.md §6 step 3** reads the instance tokens.
- **The owner example** serves its templated operations since 0.9's fix,
  so the 0.8 round's two XFAILs are plain checks. Behind R1 it also runs
  operations.md §2 steps 1 and 4, and an unbound fan-out (`internal`).

**Since 0.10.**
- **The descriptor's additions (§3.3).** The owner writes `optional: true`
  for an optional role, and states `meta.zid`. The runner checks both on
  both owners. The owner example serves `zs.thresholds.v1`, whose role
  `desired` is optional, behind R1.
- **The tool rules, where the bus shows them** (run `tool-rules`, beside
  the S1 run):
  - `live.attribute_stamp` attributes a state stamp by `meta.zid`: `owner`,
    `foreign` or `unattributable`. Since 0.11, zids compare by value only,
    and a `meta.zid` that is not hex is unattributable.
  - `live.check_s4` reads 0.11's two selectors, `@/*/router` and
    `@/*/router/**/storage_manager/storages/**`, with `plugins` beside
    them.
    - With the admin space off, the check is unobservable.
    - With it on, read-only, a router with no storage is clean.
    - **Who answered (0.12).** An answer counts only when its replier id
      (`Reply.replier_id`, which zenoh-python 1.10.1 exposes) is the zid
      its key names, and that zid is a verified router.
      - Verified routers grow outward (0.13): the session's routers and
        the session itself, then every `router` session a verified
        router's own document lists, until nothing new is added
        (`live.verified_routers`).
      - Any other answer is unverified, listed with why, and never makes
        the check clean or broken.
      - `trust=True` is the operator's alternative.
      - security.md §3 steps 1–2 run with the admin space off and on: the
        spoof on R1's own key stays unverified.
    - A client playing a storage manager is the spoof too. Trusted,
      `telemetry/**` is clean and `zk2/**` breaks S4.
    - security.md §3 step 4: with two linked routers, a client tool on R1
      verifies R2 through R1's document, and reads clean. A client
      answering on its own key stays unverified. A peer tool connected to
      both routers reads the same verdict directly.
  - `live.presence_faults` reads presence shapes twice, a grace apart. A
    stray token removed within the grace passes, and one kept is a fault.
  - `compat.classify_pair` orders two revisions met on the bus by their
    descriptors' minor, or classifies both ways. Two zk2py owners serve
    `zk2py_bringup.v1` at minor 0 and 1 (`interop/rev/`).

**Access control (0.14, 0.15).** `zk2py.acl` is a grant generator written
from §11.1–§11.2 alone. Its input is zk2py's own format, as §11.1 "The
input" allows (0.15). It is a list of `Principal`s, each bound to a usrpwd
user, with:
- the service it owns and the contracts it implements;
- `Use`s for Consume and for Call;
- the services it inspects;
- whether it reads the admin space.

A principal with no service is a pure tool; since 0.27 Tool is a role,
which a principal owning a service may hold too. `acl.generate(principals,
"deny" | "allow")` returns zenoh's `access_control` block:
- **under `deny`**, allow rules, one per (principal, key set), carrying
  §11.2's message and flow pairs (0.15's table, which zk2py measured
  first);
- **under `allow`**, denies of each grant's complement, over the
  deployment's own keys (§11.2, 0.15), with a `complement_partial` warning
  for an R2-narrowed grant;
- **fan-in:** a provider's egress and ingress `reply` carry every reader
  selector that intersects its keys. A presence selector (an inspector's,
  the doctor's wildcards) joins the egress alone (0.27), since "a
  descriptor is answered with a value, checked against its own key".
  `presence_egress=False` leaves it out, for security.md §2's check;
- **shapes compose** (0.27): Tool is a role. A principal holds the union
  of its grants, so one that owns a service and inspects others gets both;
- **the `@/**` queryable deny** is a rule in each principal's own policy
  under `allow`, never a catch-all subject's;
- **the Tool's admin read** (0.15): `query` on `@/*/router` and
  `@/*/router/**`, and their `reply`;
- **an event's keys** are its occurrences, `…/events/<template>/<ulid>`
  (core §2.6): a binding or a resource of kind event grants its template
  and one chunk more (fixed in the `config.v1` round, which first granted
  one).

`zk2py.acl_interop` (`--only acl`) runs a deployment of:
- two owners;
- a consumer;
- a caller, and the same caller with its presence removed;
- a Tool with the admin read;
- an enrolled `S` holding only the open contract grants;
- a client `S` that is no principal.

R1 is a zenoh-python router with usrpwd and the generated block. Each
posture and each variant of security.md §2's generator check runs on a
router of its own. It shows:
- security.md §1 and §2's expectations;
- §3 step 3 as amended in 0.15, with `S` enrolled and not, under both
  postures;
- the Tool's admin read under `deny`: the tool runs S4, and a principal
  without the admin read gets nothing;
- 0.14's measured facts:
  - a value reply is checked against its own key, a refusal against the
    query's;
  - under `allow`, an ungranted wildcard GET gets nothing back, while a
    wildcard call to a `fanout = "allowed"` operation still executes;
  - presence grants read the descriptor;
- §11.3's link refusal of an unauthenticated session;
- **a tool's `inspects`** (0.27): a tool inspecting `*/tc`, binding and
  calling nothing, reads both backends' tokens and descriptors on the
  doctor's wildcard selectors, none of their data, and no presence it
  does not inspect. Without the presence selectors in the providers'
  egress, its descriptor read gets nothing while its liveliness read
  still holds the tokens;
- **one principal, both roles** (0.27): `ops/both` owns a service and
  inspects `*/tc` with the admin read. It puts under its own prefix and
  not on h1's key, and reads `*/tc`'s presence and the admin space.

S4's storages read keeps only answers whose key the selector includes,
ending `…/storage_manager/storages/<name>` (§4.2, 0.15). A router's
`router/queryable/<…/state/**>` records intersect the selector.

**Since 0.16 and 0.17** (run `0.16`, and the `acl` run under `deny`):
- **A tool's S1 check (§4.2)** is `live.s1_check`. It judges the stamp
  first, then compares `meta.zid` with the zids it knows to be routers,
  by value. Its poles:
  - a foreign stamp is a finding, whether or not a router is verified;
  - an owner in client mode under a verified router is clean;
  - an owner opened in router mode and linked to R1 (`Owner(router_connect=…)`)
    is unobservable;
  - with no router's own answer counted, an owner's own stamp is
    unobservable (0.18).
- **O3 judged from outside (§5.1)** is `live.o3_verdict`. It takes the
  grants from the deployment (`acl.may_call`) and the owner's presence.
  - A silence is unobservable when the tool was not told its grants, or
    was told they do not let it call.
  - A granted call is clean.
  - A granted call that a present owner leaves unanswered is the finding.
  - `CallResult.silent` no longer counts zenoh's `Timeout` error reply as
    an answer (§5.1).
- **U22's tokenless set** is `Owner(tokenless=…)`: `"token": false`, no
  interface token. `archive.v1` in it is refused at step 2, whatever the
  owner implements (§4.4, §8.2 step 2). presence.md §2 step 6 runs in the
  refusal run, and the descriptor checker reports D011 whatever the
  entry's `contract` says (cascade 6, 0.18).
- **§2.6's replay bound** is `live.replay_events`. It GETs with `_time`
  and filters by the ULID's time, so the consumer applies the retention
  even against a storage that ignores `_time`. zenoh-python routers run no
  storage manager, so the storage's own pruning is not measured.
- **Appendix B.** `Reply.replier_id` is unstable in zenoh-python 1.10.1:
  it is marked in the stub, and present at run time. With it absent
  (simulated by `live.READ_REPLIER = False`), every admin answer is
  unverified, and S4 and S1 are unobservable, never clean.

**Known deviations:** none. `hostid.v1` 0.3 resolved F-99, and the owner
example now logs its ephemeral start at WARN on stderr. The runner keeps
the XFAIL/XPASS mechanism for a rule the owner example does not meet yet.

The runner adds a third Rust-owner run, on `interop/zk2py_echo.v1.toml`,
whose state zk2py GETs and whose operations it calls. It also adds two
zk2py-owner runs:
- one read by zk2py's own client and by the Rust `consume` example (three
  times: echo, the protobuf `app` refusal, the JSON `invalid_request`);
- the refusal of presence.md §2 step 4, watched through a router of the
  runner's own with a control (`interop/zk2py_needs.v1.toml`).

Result: `live interop: 423 passed, 0 failed, 0 known deviations of the Rust owner example`,
its `config` run (`--only config`, below) 94 of them; that run is 97 since
three of `config.v1` 0.3's probes joined it (round 30, which reran it alone). Exit codes are as for the static runner. `--only <run>`
(repeatable) runs some of the runs behind R1 alone, for instance
`--only fanout --only o1`.

**§8.1's handler rule in zenoh-python.**
- Every liveliness GET passes a `zenoh.handlers.Callback`, whose callback
  only appends to a list and whose drop function marks completion.
  zenoh-python has no unbounded channel, so a callback is the only
  compliant handler.
- The tool's session holds no bounded subscriber. Measured: a bounded,
  undrained subscriber starves even a callback GET (F-47).
- A GET with any error reply (`Timeout` when it reached its timeout) is
  reported as possibly incomplete (0.8). Elapsed time is not used.

**§8.4 in zenoh-python.**
- The full fingerprint from the descriptor, never a prefix (0.8).
- Target `BestMatching`, then `All`.
- Consolidation `None`, so that each reply can be verified as it arrives.
  zenoh's default (`Auto`, which is `Latest` here) delivers one reply, at
  completion, and can drop the valid one (F-50).
- An unbounded queue fed by a callback; the first reply that verifies is
  accepted.

The live findings are F-46 to F-55 in `SPEC-FINDINGS.md`.

## hostid.v1 (profile text 0.2, core 0.20)

```bash
just py-hostid
```

`hostid.v1` is the first profile (`spec/profiles/hostid/`). zk2py takes it
in as described below.
- **The `hostid` conformance family:** §2.1's derivation and §2.11's
  shape, 42 of 42.
- **The runtime half, `zk2py.hostid.Runtime(root)`,** one per process, over
  an injectable root, so that nothing touches `/etc`:
  - the input ladder of §2.4: absent, refused, unreadable, and a 4,096-byte
    bound;
  - paths resolved under the root as a chroot would (0.2): an absolute
    link target starts at the root, and `..` stops there;
  - the shared file of §2.5: an exclusive temporary file, `fchmod` 0644,
    `fsync`, `link(2)`, a directory `fsync`, and the temporary file always
    unlinked;
  - `HostidError`, naming every path with its outcome;
  - the ephemeral opt-in, logged at every start of a service whose system
    is ephemeral. A racer's file absent after `EEXIST` fails closed, even
    ephemeral (0.2);
  - minting once per run. The first service that asks fixes the setting,
    whether or not it starts, and a failure mints nothing (0.2);
  - §2.3's `address = "@hostid.v1/<service>"` with its configuration errors.

  Its seams are `link` and `random_bytes`.
- **The owner:** given `hostid=Runtime(...)` and the system `@hostid.v1`,
  it mints before its session opens. Its descriptor then lists `hostid.v1`
  and states `meta.host`.
- **The tool:** `hostid.minted_by_listing` answers §5's first question.
  `live.hostid_collision` answers §2.12's question, counting only the
  instances whose first answer is yes (0.2). It reads their contracts' `uses`
  by the fingerprints their descriptors name.
- **`self.system` providers (core R1, 0.20):** the owner resolves them at
  start, from the same system as its address (`owner.resolve_providers`),
  and lists them resolved. A tool refuses one. `role_keys` and
  `subscribe_role` read through the resolved bindings, discarding a sample
  on a wildcard key (R6).
- **`python -m zk2py.hostid_scenarios`** runs scenarios.md §1 to §6 in
  temporary roots, 34 of 34, with 0.2's new steps §1.6, §1.7, §4.6, §5.6
  and §6.5. §5 step 2's `logger` binds `self.system/sysinfo`.
  - §2's racers are 16 processes.
  - §3's root-only cases come from `chmod`, since the runner is no root, and
    from the `link` seam.
  - Where a section expects the bus, it uses an in-process zenoh-python
    router, with no Rust owner.
  - §5's re-mint is a new owner in the same process, since zk2py's owner
    has no re-mint.
- **Across the two implementations** (`just py-live`, `--only hostid`):
  - the owner example minted over a root holding M1 is `h-bbd1aa1db10b`.
    Its descriptor lists `hostid.v1`, `meta.host` and `meta.zid`, with its
    `profiles` in §9.5's order, and M1 in no spelling;
  - a zk2py consumer minted over the same root has the same system. It
    binds `self.system/echo` and `self.system/*`, lists them resolved,
    reads the Rust owner's state and calls its operation through them;
  - the shared file is created by one implementation and read by the
    other, both ways, with one system. Both at one minted address are
    §2.12's finding, and `no` once one leaves;
  - the owner example fails closed on an unwritable root, naming the three
    paths, and with `--hostid-ephemeral` starts and writes nothing;
  - bindings.md §5, with zk2py's detectors and trackers.

## freshness.v1 (profile text 0.2, core 0.21 to 0.23)

```bash
just py-freshness
```

`freshness.v1` is the second profile (`spec/profiles/freshness/`), and the
first to publish an annotation vocabulary: `freshness.ttl_s`. zk2py takes
it in as described below.
- **The `freshness` conformance family:** `horizons.json`, `judgements.json`
  and `clock-trust.json` (0.2), 102 of 102.
- **The linter:** W105 reads a profile's published table before its
  interim one (core 0.21). Two majors of one profile in one contract's
  `uses` are E002 (core 0.22).
- **`zk2py.freshness`, the session-free half:** `horizon` (§2.1–§2.3),
  `judge_subscription` (§2.5), `judge_get` (§2.6, the band),
  `judge_observation` (§2.7's order of checks), `combine` and `judge`
  (§2.7), and `resource_verdict` (§5's second question). Ages are integers
  of nanoseconds.
- **The runtime half:**
  - the owner re-puts every state member whose ttl is above 0, unchanged
    under a fresh minted stamp, at most ttl/2 after its last put (§2.4). It
    stops for a deleted member, a closed writer (`close_writer`), and
    while its clock guard holds (`heartbeat=`, core §4.3, §2.10);
  - `Subscriber` ages members on its monotonic clock from its declaration
    (§2.5);
  - `ClockTrust` is a GET reader's trust in a stamping clock: the
    deployment's word, or the live puts measured over one reading: the
    offset closest to zero within the delta, and none ahead beyond it (§2.6,
    0.2);
  - `get_reading` is the S4 GET, judged at a later instant;
  - `read_service` is a tool's verdict per resource, as scenarios §6 reads.
- **`python -m zk2py.freshness_scenarios`** runs scenarios.md §1 to §6, 21
  of 21, on an in-process zenoh-python router. The owner, S and G are its
  clients. A skewed clock is the owner's offset.
- **Across the two implementations** (`just py-live`, `--only freshness`):
  - the owner example serving `beacon.v1` re-puts `status` on its own.
    zk2py's subscriber and GET reader judge it fresh, and the GET's stamp
    is the latest re-put's (core S2, 0.21). zk2py's tool reads its bundle
    and gives one verdict per resource;
  - stopped with SIGSTOP, it is stale to the subscriber while its tokens
    are present, both reported. Continued, it is fresh again. Closed, it
    is stale with no tokens;
  - zk2py's owner of `zk2py_fresh.v1` re-puts, and the Rust `consume`
    example reads two different re-put stamps 1.5 s apart, then one once
    the writer closes.

## health.v1 (profile text 0.3, core 0.23 and 0.24)

```bash
just py-health
```

`health.v1` is the third profile (`spec/profiles/health/`), and the first
with a standard contract: `health.v1.toml`, moved from the walkthrough
unchanged and published in `spec/profiles/.history`. zk2py takes it in as
described below.
- **The `health` conformance family:** `judgements.json`, `rollups.json`,
  `codes.json` and `clock.json` (0.3), 112 of 112. The examples family walks the profiles'
  standard contracts against `spec/profiles/.history` too.
- **`zk2py.health`, the reader's half:**
  - the levels, unknown kept apart (§2.1, §2.6);
  - `judge`, §2.11's procedure, the status's freshness judged by
    `zk2py.freshness` at 60 s;
  - `rollup`, `code_class`, `status_agrees` and `clock_ahead` (§5);
  - proto3 codecs for `Status`, `Check` and `Fault`;
  - `read_near` and `read_across`, a reading on a bus.
- **`HealthOwner`, the owner's half,** over zk2py's owner:
  - the status put before step 4, re-put by freshness, never deleted
    while the service runs (§2.3);
  - checks put on change and deleted when retired, with §2.2's rule and
    the order of the puts. A status better than a current check is refused;
  - faults with codes of §2.10's form;
  - `clock_ahead` when the guard first holds, every 30 s while it holds,
    and none after it releases (§2.5).
- **`python -m zk2py.health_scenarios`** runs scenarios.md §1 to §8,
  29 of 29, on in-process zenoh-python routers. §2, §4 and §8 wait out the
  60 s horizon. §6's archive is a stand-in, and §8's face is a router with
  usrpwd and access control.
  - §4 runs 0.2's two tiers. Without root (`--only 4`) the fault's stamp is
    not checked. In [moved clock] (`--only 4m`), an owner client without an
    HLC (`Owner(hlc=False)`) stamps from its offset clock, so R1 re-stamps
    its fault. Step 4 runs under `drop_future_timestamp`.
  - §5 to §7's tool T reads by presence and GET, on the deployment's word
    for its clock (0.2).
- **Across the two implementations** (`just py-live`, `--only health`):
  - zk2py reads the owner example's health (`--health`). Its puts are
    driven over its stdin, and their order is seen by zk2py's subscriber.
    T judges after each command;
  - its faults are seen, and a malformed code is refused;
  - its status is re-put, and goes stale after `confirm off`;
  - `clock_ahead` under `--clock-offset-ms 2000`, in the [moved clock]
    tier, re-stamped by R1, with the status stale at 65 s. After
    `clock-offset 0` there is a re-put and no fault;
  - §4 step 4 runs under `drop_future_timestamp`;
  - the Rust `consume` example reads zk2py's health status, and stops at
    its operation step, since `health.v1` has none.

## The population budget (core §2.7, 0.24, 0.25 and 0.27)

- **The `budget` conformance family:** `bounds.json`, `population.json`,
  `rate.json` and `lossless.json`, 95 of 95.
- **Lost no delivery is established** (0.25). `budget.lossless` takes the
  resource's declared QoS, the reader's own loss and miss detection. Only a
  `reliable`, `block` resource's window can be lossless. A `Window` counts
  its span from its declaration plus the reply timeout, and watches the
  owner's token until one lease after its end.
- **Its reader lost none** (0.27): no queue loss, no transport of its
  session closed, and no gap beyond `stall_bound` (min of
  `wait_before_close` and the lease less one keep-alive interval: 5000 ms
  by default, 750 ms for a 1 s lease). `reader_stalled` is second in the
  reasons, after `reader_lost`.
- **`budget.StallWatch`** is how zk2py reads its own stall. zenoh-python
  1.10.1 exposes transport events
  (`Session.info.declare_transport_events_listener`, unstable), so it
  records every transport of the session that closes, and a ticker thread
  (every 50 ms) records the longest gap. Every `Window` runs one from
  before its declaration until the end of its watch. zk2py states no
  premise.
- **`zk2py.budget`:** `bound`, `population` and `rate`, as the fixtures read
  them. `get_population` is a complete S4 GET, and `Window` a window over a
  stream's or an event's members, with the owner's presence watched
  throughout.
- **The owner keeps the rule.** A new live member beyond its bound (a
  templated state, a stream live for an hour, an event live for its
  retention), or an occurrence beyond its rate, raises `BudgetExceeded`.
  `cardinality=` lowers the bound in the descriptor, `budget=False` makes a
  test owner that breaks the rule, and `occur()` publishes an event.
- **`just py-budget`** (`python -m zk2py.budget_scenarios`) runs
  `scenarios/budget.md`, 41 of 41. Pass `--owner <exe>` for the Rust owner
  example; it defaults to `target/debug/examples/owner`.
  - §1 to §3 are the measurements, from Python. §1 runs routers in
    processes of their own (`zk2py.proc`), §2 uses relays that close and
    stall, and §3 uses `zenoh.ext`'s miss detection.
  - §1 (0.27): step 3's stuck callback loses samples, sees its own
    transport close and the owner's token withdrawn, and its ticker no
    gap. Step 4's consumer (`proc.py consume`) stops its whole process
    for 7 s: a gap of 7 s or more, its transport closed, the token
    withdrawn, samples lost.
  - §1, "a tool's window over a stalled reader" (`--only 1b`, 0.27): a
    tool's window over a zk2py event (`interop/budget/zk2py_stall.v1.toml`),
    1 s lease, through a relay. Closing the tool's connection mid-window
    makes it `reader_stalled`, its population and rate unobservable. Left
    alone it is within, complete.
  - §4 to §7 are the owner keeping its bound and its rate, run on zk2py's
    owner and on the owner example, the same writes.
- **Across the two implementations** (`just py-live`, `--only budget`, 16
  checks):
  - the owner example's `health.v1` checks, a templated state of
    cardinality 64, are judged from complete S4 GETs: `within` at 10 and at
    64, a 65th refused by the owner, and a GET that ran to its timeout
    `incomplete`;
  - zk2py's owners give a state with its bound lowered, a stream, and an
    event whose window of one retention, 61 s, is a complete reading. Each
    is shown both ways: a conforming owner, and a test owner that breaks
    the rule and is the finding;
  - windows by 0.25: a best-effort stream is `lossy` at any length. The
    owner example's event window is `complete`, its stream window too
    short, and a window whose owner left within the watched lease is
    `owner_absent`.

## A late joiner's seed (core §4.2, 0.26 and 0.27)

```bash
just py-state
```

- **`zk2py.seed.LateJoiner`** subscribes first, then makes S4's GET
  (`All` + `Latest`). It merges the two per key by stamp, compared as
  `Latest` compares replies: a newer live put or delete replaces a reply, a
  newer reply replaces a live sample, a `reply_del` is a deletion, and
  equal stamps are one mutation, kept once. Each arrival is recorded as
  kept, the same mutation, or dropped.
  - **The order** (0.27): by time, then by id as zenoh compares ids, the
    128-bit value written as 16 bytes little-endian and compared from the
    first. At one time, id `100` (hex) is below id `2`. `seed.compare` and
    `seed.kept` are the pure merge the `seed` fixtures read.
  - **An unstamped sample** ranks below every stamped one: it is kept on a
    key holding nothing or only unstamped samples, the later arrival
    winning.
  - **R6** binds a consumer, which discards a sample on a non-concrete key.
    A tool's seed keeps it under its own key.
  - A GET that ran to its timeout seeds possibly incompletely, and an owner
    absent seeds nothing, which is silence.
  - `order="get-first"` is state.md §10's control.
- **The owner** lets a test hold a state GET: `state_hold` takes it,
  `state_answer` makes the answer, and `answer_state` sends it.
- **`just py-state`** (`python -m zk2py.state_scenarios`) runs state.md §10
  against zk2py's owner, plus the timeout and absent-owner cases, and §11
  (0.27), 15 of 15. §11's W is a peer session without an HLC, which puts
  unstamped samples and answers a held GET with stamps zk2py chooses. The
  Rust owner example does not apply to §11: its keys are an owner's,
  always stamped.
- **Across the two implementations** (the live runner's `--only seed`, 7
  checks), over the owner example's `budget.v1` items:
  - a put between the subscription and the GET is kept once;
  - a stale reply loses to a newer live put. The runner holds the GET open
    with a queryable of its own, which answers nothing;
  - a delete reaches the merge as the owner's `reply_del`;
  - a GET that runs to its timeout, with the owner stopped, is possibly
    incomplete.

## config.v1 (profile text 0.3, core 0.29)

```bash
just py-config                                   # against zk2py's owner
python -m zk2py.config_scenarios --owner <exe>   # against the Rust owner example
python -m zk2py.live_interop --only config       # the live runner's config run
```

- **`zk2py.config`**, the session-free half, from the text alone:
  - time on the wire (§2.8): any RFC 3339 instant read, the owner's
    written to the millisecond in UTC; the window, raised to the floor;
  - values (§2.2, §2.5): the three kinds, bounds inclusive, an integer
    written without a fraction or an exponent, within 64 bits;
  - the orders of refusals: the requests' own types first, a value outside
    `ParamValue` among them (0.2), then `set`'s rows 1 to 11
    (`check_set`), the control operations' (`check_control`) and
    `persist`'s (`check_persist`, a persisted layer absent or unwritable
    before the request's type);
  - desired state refused whole, the first problem in the stated order
    (`check_desired`), and redaction (§2.15);
  - **`Core`**, the owner as a state machine: calls, device results and the
    owner's clock in; replies, device acts, change events and the view and
    status out. Rows 12 to 14 (the event's rate, a reach join's deadline,
    the device), tokens, joins, the undo once, a restart, persist,
    idempotency keys and desired-state convergence. A device result can
    hold an act in flight, and every call meanwhile is `busy`.
- **The `config` conformance family:** 319 of 319 (`validation.json` 107,
  `window.json` 18, `lifecycle.json` 26 cases checked at their start and
  after each of their 131 steps, `desired.json` 28, `redaction.json` 9).
  0.1's cases passed the first time they ran; 0.2's new ones once zk2py
  followed F-107 and F-111.
- **`ConfigOwner`**, the session half, over zk2py's owner: every resource
  exposed, `persist` listed unavailable (cause `config`) without a
  persisted layer; the view and the status put before the tokens, on every
  change of either, and re-put every 30 s; each change's event on
  `events/change/<ulid>`; deadlines and reach applies on a timer of the
  owner's own; a desired document read from a key the deployment names.
  Its device is `SimDevice`, which a test sets each act's result on.
- **The reader of §5:** `read` (presence and the descriptor, or the
  deployment's word; the view and the status judged by `freshness.v1` at
  60 s; the events heard) and one `answer_*` per question.
- **`just py-config`** (`python -m zk2py.config_scenarios`) runs
  `scenarios.md` §1 to §14 against zk2py's owner, through zk2py's operation
  server and client, 51 of 51. The long sections (§1, §6, §10, §14) run
  beside the others, each on a bus of its own; the run takes about six
  minutes. With `--owner <exe>` each section drives the Rust owner
  example's `--config` instead, a black box, through its flags and stdin
  (an adapter per owner).
  - §8 runs R1 under zk2py's generated `deny` block with usrpwd.
  - §10 drops step 1's reply at the owner, which keeps it as the key's
    first answer.
  - §12's author is a zk2py owner of `interop/config/zk2py_author.v1`, its
    `state/desired` bound to `lab/rf0` by the owner's configuration.
  - §13's face is `constrained.md` §1 step 2's attachment, as `health.v1`
    §8 runs it: C a client of the owner's router, whose access control
    denies `@zk` and the state subscription to C's principal, C's batch at
    1 KB. The link is not shaped to a serial line.
  - §14's tool is the reader above, `lab/plain` a zk2py owner of `nav.v2`.
- **Across the two implementations** (the live runner's `--only config`,
  97 checks), zk2py the caller, tool and subscriber, the owner example the
  owner:
  - its bring-up: the documents before the tokens, its descriptor and
    bundle against the standard contract, its schema;
  - the order of refusals: 40 requests to an idle owner, each predicted by
    zk2py from the served view and answered by the owner example, every
    answer matching;
  - `scenarios.md` §1 to §14, all run, 51 of 51: its device through
    `config fail-apply`, `fail-undo unknown` and `device`; restarts through
    `config restart`; §10's 300 s through `config offset 300000`; §12
    through `config desired`. It takes no credentials, so in §8 and §13 it
    sits on an open router linked to C's.

- **0.3** made "an equal request" equal as decoded (`"dry_run": false` is
  absent), read the request's integer members within the unsigned 64-bit
  range, and restores a hot group left part applied once: zk2py's device
  reports it as `{"refused": words, "partial": {…}}`.

## link.v1 (profile text 0.7, core 0.32)

```bash
just py-link
```

- **`zk2py.link`**, from the text alone:
  - the face policy (§2.1): `parse_policy` reads the TOML file and reports
    the linter's findings, each `{code, severity, face, at}`, sorted,
    `far_of_two_faces` among them (0.7);
  - `link.exposure` (§2.2): `exposure`, and whether a resource's pushes
    cross a face (§2.7, §2.8);
  - the plan (§2.3, §2.5 to §2.12): `plan` derives each face's denies, with
    §2.7's carve-out and its overlaps (`carve`), its downsampling rules, its
    attachment, its window floors and its liveness evidence, or the findings
    that refuse it. Every revision of an interface it is given is read
    together (`merge_revisions`, 0.3): a resource crosses only where every
    revision declaring it reads `link`, else `exposure_disagrees`, and its
    horizon is the least, where every one gives one;
  - the checks: a deployment's tombstone windows (`check_windows`) and a
    router's block (`check_block`, §2.15). Given its posture, subjects and
    the subjects each policy lists, `check_subjects` reports
    `near_subject_on_face`, `face_subject_beyond_face`,
    `face_subject_misplaced` and `face_subject_missing`, and
    `narrowed_too_far` reads every face and every name by its kind (0.4,
    0.5). `check_router_block` reads a router's whole block, each subject
    that can match the far principal's session checked alone
    (`subject_denies`). A block zenoh refuses at load, or an unknown face,
    raises `BlockError`. `block_disabled` reports a block whose `enabled`
    is false, or absent from a router's configuration; a missing
    `default_permission` reads as `deny` and a rule without `flows` as both
    flows (0.7, measured). `lint` is `lints.json`'s reading;
  - a generator for zenoh-python routers: `compile_faces` puts a plan's
    denies in the far principal's policy of zk2py's generated block,
    narrows every other principal's subject off the face, and under
    `allow` adds the face's own subject to the far principal's policy
    (§2.4, 0.3). `compile_face` is 0.2's, kept for §7's control.
    `router_extras` gives its `downsampling` and a south region's
    `gateway`, and `compare_block` compares a block with the plan;
  - §2.5's declaration grant for a south region (`declaration_grant`, 0.2):
    the owner's `state/**` and `@state/**` for a state GET, an operation's
    template pattern for a call;
  - §5's "Is this provider live across the face?" (`liveness_across`).
- **The `link` conformance family:** 169 of 169, every case passing the
  first time it ran.
- **`just py-link`** (`python -m zk2py.link_scenarios`) runs `scenarios.md`
  §1 to §13 as 0.7 writes them, 89 of 89, every section, on zenoh-python
  routers: N with usrpwd and the block zk2py compiles under `deny` (and
  `allow` in §7, §9, §10 and §11),
  listening on TCP (the near side) and on a `unixsock-stream` path (the
  face); G across the face, L on the near side, zk2py owners of `radio.v1`.
  - The deployment is `examples/zk2/link/`, read as given: the contract,
    the face policy (each section's changes applied to it) and the
    enrollment, read into zk2py's principals.
  - G and L subscribe to the selectors their grants name,
    `zk2/lab/*/radio.v1/<token>/<pattern>` (0.2, F-116).
  - §2's far router F names its region and attaches over the face. Its
    three grants: none, neither G's GET nor its call answered; G's own
    selectors, the call answered and the GET not; §2.5's, both answered
    (0.2, F-117).
  - §7 runs X, `local`'s credentials over the face, against five blocks
    and both postures: nothing at all under 0.3's `deny` block, the far
    principal's share under its `allow` block, `local`'s own grants under
    0.2's block (F-118's measurement), and everything with no face
    subject under `allow`. Step 5's control holds: a face subject in a
    policy of its own lets G's puts on the owner's key reach L.
  - §8 runs two owners on two minors that disagree on `air/frames_total`:
    from both revisions it crosses for neither, and from the newer alone
    for both (F-119).
  - §9 and §10 run the blocks 0.4's checks refuse, each a hand edit of the
    generator's, with their controls:
    - a subject bound to `ground` and `local`;
    - narrowings further than §2.4's;
    - a face subject beyond the face, in a near principal's policy, or in
      none;
    - empty lists, which zenoh-python refuses at load.

    §9's step 2 confirms on zenoh-python that equal subject combinations
    merge.
  - §11 runs subjects naming both kinds, F-121's blocks among them, and an
    mTLS session. N also listens on TLS with `enable_mtls`, certificates
    minted by openssl for the run. Measured: a session presenting the CN
    matches a subject naming a user name and that CN, and one presenting
    another CN, or none, does not. Its step 7 asserts zk2py's branch of core
    §11.2 exactly: the stricter complement, under which X receives nothing
    at all while a probe on a key no contract declares shows its session
    is live.
  - §12 takes out of the generator's block what zenoh defaults: `enabled`
    (false: nothing is enforced, and the tool reports `block_disabled`),
    `default_permission` (`deny`), and a rule's `flows` (both flows).
  - §13 gives the policy a second face, `sat`, with the same far
    principal. The linter warns `far_of_two_faces`, and over the radio
    face G receives no push, since `ground`'s policy carries `sat`'s
    wholesale denies too.
  - §5's face goes down as G's session closing, its merged members kept.
- **The owner's tombstone window** is configurable (`Owner(tombstone_s=…)`,
  core S3).

## alarms.v1 (profile text 0.7, core 0.34)

```bash
just py-alarms                                   # against zk2py's owner
just py-alarms --owner <exe>                     # against the Rust owner example (or ZK2PY_OWNER)
just py-alarms --only 2 --full                   # §2 at the contract's 900 s, about 8 minutes
```

`alarms.v1` is the sixth profile (`spec/profiles/alarms/`). zk2py read its
0.1 cold in round 38, its 0.2 cold and live in round 39, its 0.3 in round
40, its 0.4 and 0.5 in round 41, its 0.6 in round 42, and its 0.7 in round
43. Its standard
contract moved from `examples/zk2/` unchanged, and its revision 1.0 is
published in both history roots, one file.
- **`zk2py.alarms`**, the reader's and the tool's half, from the text
  alone:
  - **the key** (§2.2, §2.3): `derive` runs the recipe through the
    sanitizing step, FNV-1a 64 over the framed input. It reports the
    string hashed, and whether the strict recipe (`strict_key`) accepts
    the input as it is. `is_host_scoped` compares exactly;
  - **the reference** (§2.12): `key_of` builds an alarm's key from its
    system, service and alert key, slugged, and `parse_key` reads one back;
  - **the document** (§2.4): `strict_json` first (0.2), then `read`
    against the 1.0 schema, read from `spec/` by core §7.3's subset
    (`validate`), and the time range. `decode` does all three on a
    payload, and `check` / `check_payload` add the owner's errors and
    warnings;
  - **transitions** (§2.6): `Transitions`, each sample against the
    document held before it, an absent `labels` equal to `{}` (0.2);
  - **the conclusions:** `judge` answers §2.14's two questions from one
    `Reading`, with freshness by `zk2py.freshness` at the contract's
    horizon. `owner_set` gives §2.15's verdict in 0.2's order. `any_firing`
    asks "anything at or above *S*?", and `rederives` is §5's third
    question.
- **The owner half**, `AlarmsOwner`, over zk2py's owner:
  - §2.5's acts; a duplicate observation puts nothing, and resumes
    confirmations stopped (§2.7, 0.2);
  - confirmations as freshness re-puts, which zk2py's owner now makes for
    a templated state's members too;
  - the bound (§2.8);
  - §2.9's adoption from a `Record` before the tokens, deletes and
    resolves before adoption, up to the bound;
  - `load(ttl)`, the short-ttl copy of the contract.
- **The reader on a bus**, `read_owner`, and `AlarmsReader` (0.3) over
  it:
  - presence and the descriptor;
  - the S4 GET, dated by the instant it was asked, its replies included,
    made again at each re-read;
  - §2.15's window as core §2.7 counts one: from the declaration plus the
    reply timeout, ended a lease and the reply timeout before the instant
    judged, lossless by the QoS and the reader's own receive path. A loss
    (a stall, a closed transport) or a gap in an owner's presence breaks
    it until the first complete GET after it;
  - each key's reading, its latest merged by stamp, `previous_instance` by
    `meta.zid` compared by value. A loss is fed into §2.14 step 4. A held
    key that the establishing GET left out, with nothing of it since, is
    `left_out`, which heals step 4 and reads `not_held` (0.5);
  - another session's GET stamps listed apart (0.2);
  - a one-shot tool's owners by an archive's keys (`archive_form`,
    `owners_from_archive`, 0.3), and by presence (`owners_by_presence`,
    `AlarmsReader.scope_read`). Each is a `ScopeRead`, whole by its
    presence read alone, which `any_firing(scope_whole)` reads
    (`scope_incomplete`, 0.5). An archive's read cut short only sets
    `archive_short`;
  - `ChangeReporter`, §2.16 (0.4, 0.6): the answer after each change,
    against the key's last report; a key's first answer reported only when
    yes; a loss once for the scope until its last key heals ("observable
    again"), each key keeping its last report through it; a key gone from
    the scope reported rather than dropped.
  - `AlarmsReader.scope_whole`: a presence read dated by its ask, a loss at
    or after it counting against it (0.6).
- **The `alarms` conformance family:** 316 of 316. 0.1's cases passed the
  first time they ran, 0.2's once zk2py followed F-127 and F-130, 0.3's
  unchanged, and 0.5's once zk2py read `left_out` and `scope_whole`.
- **The vocabulary** (core 0.34): `alarms` is published with no key, so
  every `alarms.<key>` draws W105.
- **`just py-alarms`** (`python -m zk2py.alarms_scenarios`) runs
  `scenarios.md` §1 to §10, each on an in-process router of its own:
  - **zk2py's owner** by default;
  - **the Rust owner example** with `--owner <exe>` (or `ZK2PY_OWNER`), a
    black box, driven from its `--help` alone (0.3, F-133): `--alarms`,
    `--alarms-record`, `--alarms-rules`, `--cardinality` and `--tokenless`;
    `alarm raise <json>`, `resolve`, `confirm … on|off` and `adoption`; a
    short-ttl copy passed as a contract file. The runs here are
    `owner-ac`, the build at text 0.3;
  - **the waits:** §2, §4, §6 and §10 run on a 4 s copy of the contract
    unless `--full`, and `--quick` skips §3's 65 s wait;
  - **the exit:** 0 when every check passes, 1 when one fails, and 2 when a
    check or a section did not run, unless `--only` or `--quick` asked for
    that subset.

  | § | zk2py's owner | The Rust owner example |
  |---|---|---|
  | 1 Raise | 9 of 9 | 9 of 9 |
  | 2 Escalate, refresh, confirm | 9 of 9 at 4 s; 9 of 9 at 900 s | 9 of 9 at 4 s; 9 of 9 at 900 s |
  | 3 Resolve, then delete (+ 0.5's `not_held` after a loss) | 7 of 7 | 7 of 7 |
  | 4 Re-raise, and its confirmations | 3 of 3 at 4 s | 3 of 3 at 4 s |
  | 5 A restart mid-incident (its SHOULD by a GET at the token, 0.5) | 11 of 11 | 11 of 11 |
  | 6 A hung owner goes stale (+ §2.7's resumption) | 6 of 6 at 4 s | 6 of 6 at 4 s |
  | 7 At the bound | 5 of 5 | 5 of 5 |
  | 8 An absent owner, for a live tool and a one-shot one | 10 of 10 | 10 of 10 |
  | 9 A key that does not re-derive | 3 of 3 (a zk2py liar) | 3 of 3 (the same liar) |
  | 10 Anything firing, for a tool (+ 0.3's gap and loss, 0.4's change reporter) | 17 of 17 at 4 s | 17 of 17 at 4 s |
  | **Total, default run** | **80 of 80** | **80 of 80** |

  - **§6 and §10 at 900 s were not run.** §6 takes about 15 minutes, and
    §10's window and watch past 900 s about 15 too.
  - **§9's liar is always zk2py's.** Its subject is the tool, and no owner
    may put a document on another key.

## What it does not cover

- **The rest of the live half:**
  - archives and alignment (§4.4);
  - deletes and tombstones (S3);
  - clocks beyond minting (S7's drift);
  - O6's summaries and retries (O4). Call metadata (O7) is sent by the
    client (`live.call(attachment=…)`) and read by the owner
    (`OpCall.metadata()`), for `config.v1`'s attribution;
  - serving roles (§6);
  - constrained faces (§8.5);
  - the scenarios other than presence.md, retrieval.md and parts of
    state.md and operations.md. types.md §2 needs a renderer, which zk2py
    does not have;
  - §11's History, Archive and union-storage grants, the namespace, and
    §8.5's face declarations toward a far router in a south region: the
    generator emits none of them;
  - §8.3's budget count, and an archive's alignment as a tool sees it
    (0.10): zk2py counts no budget and runs no archive.

  zk2py's owner holds values only for parameterless state. It answers
  templated operations through the handlers and members it is given, and
  otherwise names no member.
- **Building bundles with extras.** The spec names no source for the
  documents `views.document` references (F-26). The builder refuses such a
  contract, as the reference builder does. Verification of extras is
  implemented.
- **A TOML 1.1 parser.** §9.1 (0.4) makes 1.1-only syntax E000, and
  `tomllib` is a strict TOML 1.0 reader that refuses all five constructs.
  zk2py probes the reader at load and refuses to run on a `tomllib` that
  reads 1.1 (F-41).
- **W107** runs only on request (`load_contract(..., check_file_name=True)`),
  as the fixtures load without it. The examples family turns it on.

## Dependencies

| Dependency | Version | Why |
|---|---|---|
| Python | ≥ 3.11 | `tomllib` (TOML 1.0) |
| `rfc8785` | 0.1.4 | RFC 8785 (JCS) bytes. It refuses integers outside ±(2^53−1) and non-finite floats, which matches §9.5's restrictions. |
| `protobuf` | 6.33.5 | parsing `FileDescriptorSet`s, for the classifier and the §9.7 identity. It never produces artifact bytes. |
| protoc | 3.21.12 | artifact bytes (§9.4). §9.5 "Portability" says the protobuf fixtures assume protoc 3.21.12 with `--include_imports` and without source info. |
| pip | 25.2 | installed into the venv from its own wheel |
| `eclipse-zenoh` | 1.10.1 | the live half. zk2 is specified against Zenoh 1.10.1 (§0). The abi3 manylinux x86_64 wheel is used. |

**Why the system protoc rather than `grpcio-tools`.** The spec pins the
compiler, not a Python package. `grpcio-tools` bundles whatever protoc its
grpc release vendored. The releases of the protoc 3.21 era (1.51 to 1.54,
which pin `protobuf>=4.21.6,<5`) ship wheels only up to CPython 3.11. This
host runs 3.13, and Ubuntu 24.04 ships 3.12. The system protoc 3.21.12
here, and the pinned release archive elsewhere, reproduce every fixture id.
That includes the well-known types, whose bytes depend on the `.proto`
sources the compiler ships (F-14).

**Strictness that comes from the spec, not from the libraries:**
- Python's `json` accepts duplicate members and `NaN`; `jcs.loads` refuses
  both.
- `tomllib` accepts integers beyond 64 bits; the contract reader refuses
  them, as TOML 1.0 requires (F-16).
- JSON Schema's `format` is only an annotation; the shape checker asserts
  `uint32`/`uint64` (F-17).

## Layout

```text
impl/python/
  bootstrap.py        stdlib-only venv + pip + requirements + protoc bootstrap
  requirements.txt    exact pins
  SPEC-FINDINGS.md    every finding against the spec
  zk2py/
    lexical.py        §1.2      plain chunks, interface ids, instance ids, ULIDs
    slug.py           §1.4      slug / unslug
    keys.py           §1.1      the five key forms
    templates.py      §2.2      templates, shapes, resolution, overlap
    jcs.py                      strict JSON, RFC 8785, sha256 ids, §9.5 restrictions
    shape.py                    checks documents against contract/descriptor.schema.json
    protoc.py         §9.4      protoc invocation, well-known types
    schemas.py        §7.3 §9.4 artifacts, the subset, $ref, type references
    contract.py       §9.1–§9.5 lints, defaults, canonical form, fingerprint
    sets.py           §9.2      E035, E036
    bundle.py         §9.6      build, verify
    history.py        §9.7      the history check
    descriptor.py     §3.3      the D codes
    cbor.py, envelope.py §5.2   the error envelope
    compat.py         §9.7 §9.8 the classifier, retention identity
    conformance.py              the runner
    live.py           §3.3 §4 §5 §8 presence, descriptor GET, retrieval, state GET, calls
    owner.py          §3.3 §4 §5 §8 a minimal owner
    live_interop.py             the live runner, with the Rust owner and consume examples
    acl.py            §11       the grant generator: a deployment → zenoh's access_control block
    acl_interop.py    §11       the access-control run (`--only acl`)
    hostid.py         hostid.v1 the derivation, the shape, the runtime over a root
    hostid_scenarios.py hostid.v1 scenarios.md §1–§6 in temporary roots
    freshness.py      freshness.v1 horizons, judgements, the clock trust, a subscriber, a tool's read
    freshness_scenarios.py freshness.v1 scenarios.md §1–§6 on an in-process router
    health.py         health.v1 levels, §2.11's reader, roll-ups, codes, the payloads, the owner half
    health_scenarios.py health.v1 scenarios.md §1–§8 on in-process routers
    config.py         config.v1 time, values, the orders of refusals, desired state, redaction, Core,
                                ConfigOwner, SimDevice, the reader of §5
    config_scenarios.py config.v1 scenarios.md §1–§14 against zk2py's owner or the Rust owner example
    link.py           link.v1   the face policy, exposure, the plan and its carve-out, the checks, a generator
    link_scenarios.py link.v1   scenarios.md §1–§13 on zenoh-python routers, over examples/zk2/link/
    alarms.py         alarms.v1 the key and the sanitizing step, references, strict JSON, the document, transitions,
                                the conclusions, AlarmsOwner, the reader on a bus
    alarms_scenarios.py alarms.v1 scenarios.md §1–§10 against zk2py's owner or the Rust owner example
    budget.py         §2.7      the population budget: the bound, a population, a rate, lossless, the stall watch, readings on a bus
    budget_scenarios.py §2.7    scenarios/budget.md §1–§7 and §1's stalled reader: the measurements, and the owner keeping its budget
    seed.py           §4.2      a late joiner's seed: subscribe, then S4's GET, merged by stamp; the pure merge
    state_scenarios.py §4.2     scenarios/state.md §10 against zk2py's owner, and §11's open cases
    proc.py                     helper processes for the measurements: a router, a publisher, a consumer that stops
  interop/            zk2py's own interop contracts: probe, echo, needs, bringup, tc, scan, sysinfo,
                      sysinfo_x, tracker, order, order_b, budget, plans; freshness/ holds beacon.v1 and zk2py_fresh.v1, budget/ budget.v1 and zk2py_stall.v1; config/ zk2py_author.v1;
                      rev/ holds bringup's minor 1; stand-in/ an archive.v1 id (§4.4)
```

The JSON schemas are read from `spec/` at run time (`shape.py`), not copied.
So the E000 and D000 checks follow the published schema files exactly.
