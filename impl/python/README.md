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
| descriptors | §3.3 | `descriptor` | the D codes (D012, a standby marked tokenless, since 0.39), against `descriptors/contracts/nav.v2.toml`. That contract's fingerprint, computed here, is the `sha256:fea2…` the fixtures expect. |
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
| telemetry | profiles/telemetry §2.1–§2.11, §3 | `telemetry` | every file of `spec/profiles/telemetry/conformance/`: `schema.json` (the standard schema's published ids, the file's header against zk2py's tables, and each case's document by path, inside a bundle, or inline), `declarations.json` (a resolved resource's declaration), `samples.json` (one sample's conformance, and the member's kind and unit; since 0.2 a sample may come as its bytes, `json_text` read as strict JSON, `cbor_hex` as CBOR, `attachment_text` as JSON) and `continuity.json` (a pair's continuity, its epoch's evidence and its rate, compared exactly; `stamp_ntp64` read as zenoh's `as_nanos`, 0.2). An unknown file there fails. |
| desired | profiles/desired §2.2–§2.10, §2.17, §3 | `desired` | every file of `spec/profiles/desired/conformance/`: `schema.json` (the standard schema's two published ids, the file's header against zk2py's tables, each case's document by path or inline), `declarations.json` (a resolved resource with its parameters' types -> its declaration, or its reasons in order, `not_string` since 0.2), `bindings.json` (a requirement as a descriptor lists it -> the slice's key, the kill switch, or its findings), `order.json` (one role's steps through the target's state machine, a put's payload decoded, as its text or as its bytes: each step's word and the report after the last, which must itself read with no finding), `reports.json` (a report decoded or as its bytes: readable, and its findings in order), `conclusions.json` (§5's four questions over one reading, the deployment's word across a face since 0.2) and `slices.json` (0.5: a tool's reading of one role before §2.17 answers -> its candidate slices, where they came from, §2.3's and §2.2's problems, and which reply is the report). An unknown file there fails. |
| redundancy | profiles/redundancy §2.2–§2.15, §5 | `redundancy` | every file of `spec/profiles/redundancy/conformance/`: `declarations.json` (a contract's `uses` and resolved annotations, and the tokenless set -> governed with its election, not governed, or its reasons in order), `descriptors.json` (one entry's mark and §2.14's findings, `malformed` alone), `elections.json` (one read from the instance's view -> take over, or stand by and why), `heals.json` (two reads' holders -> step down to the lower, or stay and why), `lifecycle.json` (one candidate's events through the state machine -> each step and its role) and `conclusions.json` (§5's four questions over two reads a grace apart). An unknown file there fails. |
| timing | profiles/timing §2.2–§2.7, §5 | `timing` | every file of `spec/profiles/timing/conformance/`: `declarations.json` (a resolved resource, its annotations or its three layers, or a requirement -> not this profile's, ignored, invalid with its reasons in order, or declared with `below_period`), `deadlines.json` (one reader's tracker for one provider and resource, driven by its events, each instant read exactly to the nanosecond -> its reports, what it holds, its next due), `lifespans.json` (one member's samples within one reading -> each sample's judgement, a clock falsified for the reading), `periods.json` (one member's arrivals over a window -> kept, not kept, too short or not heard, with its count and span) and `conclusions.json` (§5's three questions of one member over one window). An unknown file there fails. |
| arbitration | profiles/arbitration §2.2–§2.11, §2.14, §5 | `arbitration` | every file of `spec/profiles/arbitration/conformance/`: `declarations.json` (one role and the revision of its required interface held -> not arbitrated, arbitrated with its policy, stream, deadline and period, unread, or invalid with its reasons in order), `rankings.json` (a binding and the addresses heard -> the providers in rank order, the unbound, the wildcard and shadowed entries), `arbitrations.json` (one consumer's role and its events -> after each, its switches, the active provider, its due, a sample's outcome, the reads again) and `conclusions.json` (§5's three questions over a tool's reading). An unknown file there fails. |
| readings | | `alarms`, `telemetry`, `desired`, `redundancy`, `timing`, `arbitration`, `presence`, `live` | not a fixture family: zk2py's own cases, each naming what it reads (an open finding's reading, or a stated rule beyond its fixture). Today, 520. Of `timing.v1` 0.5, 6 (round 81): a known member missed with its owner present, then its owner gone, held to the window's end while it stays missed, its deadline `no`; one paused by its owner's absence, then heard while its owner is absent, a new stretch from that arrival, 2,000 ms, `met`; a member ended in the window and not heard again keeping its held time, its first end, `ended`; a run's roll-up handed its members in row order reading its reason in byte order (F-279); and, after review-bot on PR #941, the roll-up's two forms explicit (`rollup` takes `(key, Answer)` pairs alone, `rollup_answer` answer tokens alone, each refusing the other's and a mix) and a tool's window closing once (a second `end` adds no report). F-278's and F-280's readings went: `deadlines.json` #46 to #51, `periods.json` #21 and `conclusions.json` #50 hold them. Of `timing.v1` 0.4, 1: where no tracker is held, an end before the member was first held in the window ending nothing (review-bot on PR #939). Of `arbitration.v1` 0.3, 3: a view governed only when every revision held lists `redundancy.v1`, listening counted from the last subscription, and a view 100 ms after its subscriptions, its watch's 4 s round trip before them, `listening` and never `dead_man`. Of `timing.v1` 0.3, 5: a tool's known members (templates without parameters), a loss it cannot attribute counted for every member, the `meta.zid` across a face on the deployment's word alone and away from one the descriptors', and F-277's roll-up, since text 0.4 (round 80) read as 0.4 writes it: every member set aside not asked, with the first not-asked member's reason, or not this profile's when every one is, none at all unobservable, with no reason since 0.5 (F-279), and a set-aside member beside one pole changing nothing. F-275's reading went: `deadlines.json` holds it since 0.3. Of `arbitration.v1` 0.2, 2: the counts line every provider known in rank order, and §11's governed `steer.v1` minor 1 loading with the reference CLI's fingerprint, `rudder.v1` arbitrated over it. Of `timing.v1` 0.2, 5: F-261's two orders through an epoch tracker (a read before any member token seen ending nothing, the wait from the token seen), F-260's `beyond_bound` once per stay past the bound, F-259's trackers, and F-275's epoch member waited for through its owner's absence. Of `arbitration.v1` 0.1, 13: the walkthrough's `thruster.v1` over `twist_cmd.v1` arbitrated and lint-clean, its enrollment's two bindings exact, the scenarios' `rudder.v1` over `steer.v1`, the linter's checks with and without the required interface, and F-268's two orders through the arbiter; and six from review-bot on PR #912: a tick just past the due moving the state in every state `arbitrations.json` reaches, the round trip after a move asked until one stands (a later move ending it, a cut-off waiting for a link), and a binding entry that is not one refused. Of `timing.v1` 0.1, 17: the adopters (`twist_cmd.v1`'s `cmd`, `nav.v2`'s `position`, `pace.v1`'s two streams) declaring as the text reads them, each lint-clean; the profile's linter's four warnings; §5's roll-up by each question's finding beside the sentence as written (F-264); a missed member staying missed when its owner is present again (F-262); a known epoch member ended by a read before its member token, as 0.1 read it (F-261; its read after the token is 0.2's now); a `meta.zid` of 33 digits stating none. Of core 0.42's §8.1, 15: a watch session in no namespace on the wire keys (the wire selector and its round trip's key, a namespace with a slash at either end read as its chunks on the wire and stripped alike, each key heard stripped, one outside the namespace not the deployment's, `Stripped` counting it and handing on the rest, `watch_or_beside` in a namespace declaring on the wire key and making no GET on the reading session, the caller's drop hook and callback mode kept), zk2py's follower over fake sessions (its watch session cut off: no round trip and no read asked on either session, the read not made with the watch session's reason, what it would establish unobservable; linked: the round trip then an ordered read; the watch session's links gone at the read's end: unordered; in a namespace, without history on the wire key, keys stripped and one outside counted, never folded; handed a namespaced watch session, 0.41's history), and a read or a round trip on a session whose interests timeout zk2py cannot read saying its completeness rests on zenoh's 10 s. Of core 0.41's §8.1, 28: a read whose time reached its session's interests timeout possibly incomplete, saying so, and what zk2py records of a session (its interests timeout, 10 s for one it did not open; its namespace); the round trip's key under the subscribers' subtree, and when it stands (its final reply, no error reply, before the interests timeout, a link before and after; none made through a session cut off, nor one whose links cannot be read); the view's unordered reads (what they show up, what they do not unobservable, never down, a withdrawal keeping its reason, an ordered read owed after any of them) and a transport's move (what the reader held unobservable, a read owed, a withdrawal keeping its reason). Of `desired.v1` 0.8, 4 of a revision that does not read beside an unread requirement (no resources list shows nothing: unread, never `not_a_target`; a readable held revision declaring no desired resource beside one that does not read, `not_a_target`; read, a revision that does not read is not in hand, so the target alone). Of `redundancy.v1` 0.2, 5 of the kept events decided partway (F-199: a read deciding take_over keeps the appearances through the act and drops the withdrawal, one deciding step_down the reverse, a heal read ending the heal with three kept re-entering the rest in `heal_first`, and over every state reached under both elections each step re-entered asking nothing). Of `desired.v1` 0.4's §2.17, 6 of how long behind (F-211: every reading over the window, another answer or version starting it again, one reading enough only on a trusted clock). Of `desired.v1` 0.4's tool, 9: §2.10's choice of the report among a target's replies (the readable one claiming the slice, two differing unreadable, one twice one, an unreadable one beside none readable claiming it, a readable one beside an unreadable, none claiming), and §2.17's slice from the target alone (the requirement's template and binding, the kind token from the report's key, a parameter left unbound). Of core 0.38's §8.1, 3 more of `list_presence`'s guards: the check reads the links, a closing transport still listed notwithstanding, and says a read was cut off while it ran. Of desired.v1 0.3's live half, 20: zk2py's author's encoding (nine numbers written by their exact values and read back the same, members sorted, a non-finite number refused) and nine of §2.4's revision read exactly. Of core 0.37's §8.1, 28: zk2py's presence view (a withdrawal unobservable and owing a read, down only from a complete read, up again from one, unconfirmed still after a read not made or possibly incomplete, a re-mint owing none, one read for a burst, a reply sent before a withdrawal not bringing it back, each key's latest move since a read was asked deciding it, a withdrawal heard back owing no read, an address never heard of down or unobservable by the latest read over it, the three holders), a read's three kinds, and `list_presence`'s guards on fake sessions (a transports accessor that fails, before or after, leaves a read possibly incomplete; the registry prunes closed sessions, never hands an entry to a session with a reused id, and closes a closed reader's watch session; since 0.38 the accessor is the links). Of `desired.v1`, 73: Appendix B over the adopters' real contracts and bindings (the examples' eight desired resources, the walkthrough executor's slice, `zs.desired.v1`'s seven templates, `zs.thresholds.v1`'s requirement bound for the sysinfo sensor and unbound, its `applied/{topic}` no Report), the scenarios' slice `D`, zk2py's own refusal of a target address that is not `<system>/<service>` (five), zk2py's readings of round 52's F-162, F-163, F-166, F-167 and F-168, now 0.2's rules (the 16 ids remembered and the 17th forgetting the first, a restart forgetting every id, exact values from a payload's text, 256 and 257 deep), of the open F-181 (a report's `18446744073709551615.0` read as its double) and F-184 (an owner's waiting version re-delivered is the same version), §2.7's equality over values strict JSON cannot yield and over exponents of 10^9, 10^23 and 100,000 digits, each finishing at once, finiteness at the edge 2^1024 − 2^970 written four ways, §2.10's report puts, and a report's bytes no fixture gives (nested 100,000 and 300 deep, a byte-order mark, a lone surrogate). Of `telemetry.v1`, 66: Appendix B over the adopters' real contracts (zenoh-modem's 73 declared streams and 23 units, `sdu/dropped_total` a bare counter with no epoch, each ZenSight contract's revision 1.1 and both published bundles a Point declaring no kind, netring's two units, tcgui's resources none this profile's), three of §2.3's copies (a JCS-equal one keeps the id; a changed one is not the Point), two of F-148's state order, 14 of F-149's `null` members (a Point's `value`, each kind's Value, each Histogram member, and `epoch`, `unit` and `labels`), and four of 0.2's open F-150 (the double's edge, read as IEEE 754 rounds it) and F-151 (an Epoch attachment's lone surrogate, read strictly), three samples nested 100,000 deep, read as undecodable, never a crash, sixteen of §5's over many pairs and samples (`verdict`, per member since 0.4, over unobservable samples since 0.5), F-152's reading and F-157's among them, five of CBOR tag chains (an outer tag around a converted bignum, 100,000 tags), and three of §2.9's seed folded into a subscription's moves, the subscription's word winning. Of `alarms.v1`, 137: 11 of `alarms.v1` §2.4's whitespace (RFC 8259's `ws` around a payload's value reads; anything else after it, and a byte-order mark before it, is `json`), six of §2.15's window healed only by a GET asked after a break was seen, two of 0.5's whole scope (an archive's read plays no part), 86 of §2.16's change reporter (over every `transitions.json` case, every `conclusions.json` reading, four loss sequences, four of 0.6's first answers, and four of a key gone from the scope), five of 0.6's presence read dated by its ask, six of §2.14 step 5's left-out key, a `reply_del` among them, and 21 of §2.16's two closed lists: each of the twenty neither reasons the first question gives, read from a reading of its own, on its list, and the table's reader's-sight reasons 0.8's ten. |
| examples | §9.6–§9.8, §10 | | Every `examples/zk2/**/<name>.v<major>.toml`, and every profile's standard contract under `spec/profiles/` (0.23): loads with **no finding at all**, W107 included; its built bundle verifies; it is published in its tree's history root (`examples/zk2/.history` or `spec/profiles/.history`), **byte-identical** to the bundle zk2py builds; it is `compatible` with its history. Both roots pass the §9.7 check, and a revision held in both is one file, byte for byte. |

The result at the time of writing:

```text
layout         24 passed     0 failed
keys           55 passed     0 failed
slugs          42 passed     0 failed
templates      11 passed     0 failed
contracts     109 passed     0 failed
sets            4 passed     0 failed
bundles        24 passed     0 failed
history        10 passed     0 failed
descriptors    48 passed     0 failed
errors         45 passed     0 failed
compat        100 passed     0 failed
budget         95 passed     0 failed
seed           20 passed     0 failed
examples      108 passed     0 failed
readings      520 passed     0 failed
hostid         42 passed     0 failed
freshness     102 passed     0 failed
health        112 passed     0 failed
config        330 passed     0 failed
link          172 passed     0 failed
alarms        316 passed     0 failed
telemetry     281 passed     0 failed
desired       408 passed     0 failed
redundancy    180 passed     0 failed
timing        196 passed     0 failed
arbitration   120 passed     0 failed
total        3474 passed     0 failed
```

The figures are against `core.md` 0.56, `hostid.v1` 0.7, `freshness.v1`
0.4, `health.v1` 0.4, `config.v1` 0.5, `link.v1` 0.8, `alarms.v1` 0.8,
`telemetry.v1` 0.5, `desired.v1` 0.8, `redundancy.v1` 0.5, `timing.v1`
0.5 and `arbitration.v1` 0.3.
`timing.v1` 0.5 added six `deadlines.json` cases (#46 to #51, a tool's
window closed by an `end` event, with `expect.held_ms`), eight
`conclusions.json` cases (#43 to #49, roll-ups by `rollup.members` with
`expect.reason`, and #50) and `periods.json` #21 (196). The runner crashed
on the first `rollup.members` case until it read them; once zk2py's
tracker kept each member's longest stretch and its roll-up read the first
member's reason in byte order (round 81), all fifteen passed the first
time they ran, `periods.json` #21 with no change. `readings` went to 518,
and to 520 after review-bot on PR #941.
`timing.v1` 0.4 added three `conclusions.json` roll-up cases and moved
one, #34 (181): F-277's, every member set aside not asked, or not this
profile's when every one is. Four failed until zk2py's `rollup` read 0.4's
rule (round 80), then passed. `readings` went to 516: round 79's reading of
F-277 is replaced by 0.4's, and F-278's and F-280's are new, F-280's two.
`timing.v1` 0.3 added three cases (178): `deadlines.json`'s F-275 (an
epoch member's wait going on through its owner's absence) and two of
`lifespans.json` across a face, which zk2py passed unchanged. `arbitration.v1`
0.3 changed no case. `readings` went to 513 (round 79).
`timing.v1` 0.2 grew `deadlines.json` to 44 cases and `conclusions.json`
to 39 (175), and `arbitration.v1` 0.2 `conclusions.json` to 24 (120):
zk2py passed them once its tracker read `epoch` and `member_seen` and
reported `beyond_bound` once, its runner read a case's `resource` and the
roll-ups, and `conclude` read `bindings` null. `readings` went to 506
(round 78). Core 0.55 and 0.56 changed no fixture.
`arbitration.v1` 0.1 added its four fixture files, 115 cases, which zk2py
passed the first time they ran, once its runner read `arbitrations.json`'s
role at the case's top level; `layout` now checks the eleven profiles'
directories that hold fixtures, and `readings` gained 7 cases (494), and
six more from review-bot on PR #912 (500).
Core 0.54 added `contracts/w105-arbitration-deadline` and
`descriptors/ok-binding-order` (109, 48), which passed unchanged: zk2py's
W105 already read a requirement's annotations, and its owner already kept a
binding's order.
`timing.v1` 0.1 added its five fixture files, 152 cases, which zk2py
passed the first time they ran; `layout` now checks the ten profiles'
directories that hold fixtures, and `readings` gained 18 cases (487).
Core 0.53 added `contracts/w105-timing-max-rate` (107), which passed on
the interim row and passes on the published one. `freshness.v1` 0.4
changed no fixture.
Core 0.48 to 0.52 changed no fixture; rounds 73 to 75's readings of them
took `readings` to 469.
Core 0.47 changed no fixture and no reading (411).
Core 0.46 changed no fixture and no reading (411).
Core 0.45 changed no fixture; `readings` gained two cases for F-242's
take-up (411).
Core 0.44 changed no fixture; `readings` gained its 15 cases (409).
Core 0.43 changed no fixture and no rule, and no reading (394).
Core 0.42 changed no fixture; `readings` gained its 15 cases (394).
Core 0.41 changed no fixture; `readings` gained its 28 cases (379).
`desired.v1` 0.8 added six `slices.json` cases, with the input
`requirement.unread` and the answer `requirement_unread` (408), which
`take_slice`, changed from §2.17 first, passed the first time they ran.
`desired.v1` 0.7 added two `slices.json` cases, a requirement naming no
resource (402), which zk2py passed unchanged: it read none as every
resource already.
`desired.v1` 0.6 added four `bindings.json` cases, with the input `also`,
and seven `slices.json` cases (400). zk2py's `bind` and `take_slice`,
changed from §2.1, §2.3 and §2.17 first, passed all eleven the first time
they ran.
`redundancy.v1` 0.3 added `polls.json`, 7 cases (180), which zk2py's
`poll_read` passed the first time they ran. Its round 58 reading, a first
poll as a baseline and a withdrawal only with no holder left, would have
failed five of them. The seven readings of that reading went with it.
`desired.v1` 0.5 added `slices.json`, 22 cases (389), which zk2py's
`take_slice` and `which_report`, written from §2.10 and §2.17 before the
file was run, passed the first time they ran.
`redundancy.v1` 0.2 added thirteen `lifecycle.json` cases (173), for
F-199, F-207 and F-209, which zk2py's machine, rewritten from the new
description, passed the first time they ran. Core 0.40 and `hostid.v1`
0.7 changed no fixture.
`desired.v1` 0.4 added eight `order.json` cases (274), with the file's
top-level `schema` and the step `{"finish": "check"}`. Each failed until
zk2py placed a refusal's pointer as §2.7 now says, then passed as
written.
`redundancy.v1` 0.1 added its six fixture files, 160 cases, which zk2py
passed the first time they ran; `layout` now checks the nine profiles'
directories that hold fixtures. Core 0.39 added five descriptor fixtures
(`ok-standby`, `ok-standby-false`, `d000-standby-type`,
`d012-standby-tokenless`, `d012-bad-fingerprint`), of which the two D012
ones failed until zk2py checked D012 (cascade 7), and
`contracts/w105-redundancy-epoch`, which passed on the interim row and
passes on the published one. Core 0.38 and `hostid.v1` 0.6 changed no
fixture. Since round 55, the fixtures
gained one `reports.json` case (266), a revision
`1e99999999999999999999`: never finite, so strict JSON refuses the text
before its exact value is computed. zk2py passed it unchanged, and
`exact_value` refuses such a number too, for a caller that skips that
read. `desired.v1` 0.3 added seven
cases (265): five of `reports.json`, F-181's exact numbers, which round
53's reader read as doubles (two failed until zk2py read them exactly),
and two of `order.json`, F-184's, which passed. `config.v1` 0.5 changed no
fixture. `desired.v1` 0.2 added 36 cases
(258) and three inputs (`params`, `deployment`, an `order.json` payload's
`text`), and published its schema's second id; round 52's reader could not
run the new `text` input, and would have read 0.2's per-id memory,
exact-value equality, `not_string` and the deployment's word otherwise.
`config.v1` 0.4 changed no fixture. `desired.v1` 0.1 added its six
fixture files, 222 cases, which zk2py passed the first time they ran, and
core 0.36 `contracts/w105-desired-target`, which failed until W105 read
`desired`'s published table; `layout` now checks the eight profiles'
directories that hold fixtures. `telemetry.v1` 0.5, like 0.4, changed no fixture. `telemetry.v1` 0.3 added twelve `samples.json` cases, two
of which (257 nested arrays, in JSON and CBOR) round 46's reader read
otherwise, the new depth rule. `telemetry.v1` 0.2 added 24 `samples.json` cases, three
of them input forms giving a sample's bytes, and two `continuity.json`
cases by raw NTP64 stamps; seven read otherwise under round 45's decoders,
the answers to F-143 and F-144. `telemetry.v1` 0.1 added its four fixture files, 240
cases and three checks of `schema.json`'s header, which passed the first
time they ran, and core 0.35 moved its vocabulary to Appendix D's published
table, which changed no fixture. `layout` now checks the seven profiles'
directories that hold fixtures. The four ZenSight contracts' revision 1.1
list the profile's file from `spec/profiles/telemetry/`, and `examples`
loads, builds, publishes and classifies each against both revisions with no
change. `link.v1` 0.8 changed wording only.
`alarms.v1` 0.7 and 0.8 changed no fixture, and core 0.34, with 0.8,
added `contracts/w105-alarms-vocabulary`. `alarms.v1` 0.6 changed only
`conclusions.json`'s description.
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
directories that hold fixtures. Core 0.33 changed no fixture. Three
`lints.json` cases came after `link.v1` 0.7, with no text change, and
zk2py passed them unchanged. `link.v1` 0.7
added 13 `lints.json` cases and changed three; core 0.32, and core 0.31
with `link.v1` 0.6, changed no fixture. `link.v1` 0.5 added ten
`lints.json` cases and changed one. `link.v1` 0.4
added 18 `lints.json` cases, and 0.3 one `plan.json` case and eight
`lints.json` cases. `link.v1` 0.2
added the scenarios' radio as an example (`examples/zk2/link/radio.v1.toml`,
4 more `examples` checks). `link.v1` 0.1 added its three fixture files,
core 0.29 `contracts/w105-link-downsample`, and `config.v1` 0.3 seven
`validation.json` cases and one `lifecycle.json` case. Core 0.28 added
three `errors` cases, and `config.v1` 0.2 eight `validation.json` cases
and two `lifecycle.json` cases. `config.v1` 0.1 added its five fixture
files and its standard contract (4 more `examples` checks), and `layout`
checks the four profiles' directories. Core 0.27 added the `seed/` family
and reshaped the `budget/` family's `lossless.json`, which 0.25 added.
Core 0.24 added the `budget/` family, and `health.v1` 0.3, with it,
`clock.json`. `health.v1` 0.1 added three fixture files, and
`freshness.v1` 0.2, in the same round, `clock-trust.json` and four
judgement cases. Core 0.22 added `contracts/e002-two-majors`. Five earlier
releases added descriptor fixtures:
- 0.19, `descriptors/ok-derivation-profile`;
- 0.18, `d011-bad-fingerprint`;
- 0.17, `d011-tokenless-archive` and `ok-archive`;
- 0.11, `ok-optional-unchecked`;
- 0.10, `ok-optional-role`.

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
- `alarms.v1` 0.8 resolved F-141: `silent` is on the reader's-sight
  list.
- A closing whole-text pass over `alarms.v1` 0.8 found nothing more to
  guess (`SPEC-FINDINGS.md`, "At alarms.v1 0.8").
- `telemetry.v1` 0.2 resolved F-142 to F-149, found against its 0.1, F-143
  partly and F-144 wholly against zk2py's reading (`1e400` and 10^400
  undecodable; a CBOR bignum an integer). The others went as zk2py read
  them.
- `telemetry.v1` 0.3 resolved F-150 and F-151, found against its 0.2,
  both as zk2py read them.
- `telemetry.v1` 0.4 resolved F-152, found against its 0.3, as zk2py read
  it, and asks it of each member. F-153 to F-155, against the tool's help
  and output, are answered in `zenctl-te`.
- `telemetry.v1` 0.5 resolved F-156 to F-158, found against its 0.4, F-157
  against zk2py's reading (an undecodable sample or an invalid
  declaration's member now leaves a window unable to tell).
- Against the text, none open: 0.5's changes were read with full care, and
  round 48's whole-text pass reached the rest. F-159, against the tool's
  across-face "listening", is resolved in TG (`zenctl-tg`), and F-160, its
  line naming a proven opening only by silence, in TH (`zenctl-th`).
  None is open: telemetry.v1's rounds 45 to 51 are closed
  (`SPEC-FINDINGS.md`, "At telemetry.v1 0.5, the tool after TH").
- F-161 to F-180 were found against `desired.v1`'s text 0.1, read cold
  with core 0.36 in round 52, its scenarios read for sufficiency only
  (`SPEC-FINDINGS.md`, "At desired.v1 0.1").
- `desired.v1` 0.2 and `config.v1` 0.4 resolved all twenty. F-163, F-166
  (partly), F-168 and F-170 went against zk2py's reading: an older version
  of any remembered id is older, every number is its exact value, the
  deployment's word answers step 2 across a face, and a target parameter
  must be a `string`. The others went as zk2py read them, or were stated.
- F-181 to F-184 were found against `desired.v1` 0.2 and `config.v1` 0.4,
  read cold in round 53 (`SPEC-FINDINGS.md`, "At desired.v1 0.2"). All
  four are open, and minor.
- F-185 to F-191 were found against core 0.37, read cold in round 54 with
  its scenarios `presence.md` §7 to §9 run from Python
  (`SPEC-FINDINGS.md`, "At core 0.37"). `desired.v1` 0.3 answers F-187's
  and F-191's `desired.v1` halves; the rest are open.
- `desired.v1` 0.3 resolved F-181 (against zk2py's reading: a report's
  numbers are their exact values), F-183 and F-184, and `config.v1` 0.5
  F-182.
- F-192 to F-198 were found against `desired.v1` 0.3 and `config.v1` 0.5,
  read cold and run live in round 55 (`SPEC-FINDINGS.md`, "At desired.v1
  0.3"). F-194 is against the owner example's help. All seven are open.
- Core 0.38 resolved F-185 to F-191, F-187's `alarms.v1` half by that
  text's §2.7 wording, its text still 0.8.
- `desired.v1` 0.4 resolved F-192 to F-198, each as zk2py read it or
  stated where it read none (`SPEC-FINDINGS.md`, "At desired.v1 0.4").
- F-210 to F-213 were found against `desired.v1` 0.4's tool rules, read
  cold, and run live with both tools in round 57. F-212 is minor, and F-213
  is where the two tools first disagreed. zk2py now reads as the
  reference does there.
- `desired.v1` 0.5 resolved all four (`SPEC-FINDINGS.md`, "At desired.v1
  0.5").
- F-217 to F-219 were found reading 0.5 cold, end to end, and asking both
  tools about shapes it does not state, in round 59. F-217 and F-218 are
  §2.17's; F-219, minor, is against the reference tool's help. On every
  shape the two tools agree.
- `desired.v1` 0.6 resolved all three (`SPEC-FINDINGS.md`, "At desired.v1
  0.6").
- F-224 was found reading 0.6 cold, end to end, every MUST against its
  evidence, in round 61. §2.1's refusal at a target's start had only a
  fixture of the binding behind it, and minor.
- `desired.v1` 0.7 resolved it: the start is refused, and scenarios §11
  observes it (`SPEC-FINDINGS.md`, "At desired.v1 0.7").
- F-225 was found reading 0.7 in round 62: a declaring contract named and
  not held is read by both tools as naming every resource. It is minor.
- `desired.v1` 0.8 resolved it: such a requirement is unread
  (`SPEC-FINDINGS.md`, "At desired.v1 0.8"). Round 64 found nothing new.
- F-199 to F-209 were found against `redundancy.v1` 0.1, core 0.39 and
  `hostid.v1` 0.6, read cold and run live, zk2py against itself, in round
  56 (`SPEC-FINDINGS.md`, "At redundancy.v1 0.1 and core 0.39"). Five
  are the machine's or the text's (F-199, F-204, F-206, F-207, F-209),
  five the scenarios' (F-200 to F-203, F-205), and one `hostid.v1`'s
  (F-208). F-205 is minor.
- `redundancy.v1` 0.2, core 0.40 and `hostid.v1` 0.7 resolved all eleven
  (`SPEC-FINDINGS.md`, "At redundancy.v1 0.2 and core 0.40").
- F-214 to F-216 were found reading them cold and running the profile
  live in round 58, against itself, the owner example, and the two
  mixed. F-214 is against the owner example's refusal, F-215 is §2.7's,
  and F-216, minor, the scenarios'.
- `redundancy.v1` 0.3 resolved all three (`SPEC-FINDINGS.md`, "At
  redundancy.v1 0.3").
- F-220 to F-223 were found reading 0.3 cold, end to end, and running
  §12 against the Rust tools in round 60. F-220 is against the owner
  example's `--wait-go`, which never starts. F-221 is §2.15's probe of a
  template-less operation. F-222, minor, is against `check conform`'s
  `--skip`, outside this profile. F-223, minor, is §3's "unless its
  interface token beat that GET", which S cannot read on a fast
  takeover.
- `redundancy.v1` 0.4 resolved all four (`SPEC-FINDINGS.md`, "At
  redundancy.v1 0.4").
- F-226 was found reading 0.4 cold, end to end, in round 63. It is §12's
  evidence for half of §2.15's new MUST, a revision held offline that no
  descriptor names. It is minor.
- `redundancy.v1` 0.5 resolved it with §12 step 9 (`SPEC-FINDINGS.md`,
  "At redundancy.v1 0.5"). Round 65 found nothing new.
- F-259 to F-267 were found against `timing.v1`'s text 0.1, the tenth
  profile, and core 0.53, read cold and run live, zk2py against itself, in
  round 76 (`SPEC-FINDINGS.md`, "At timing.v1 0.1 and core 0.53"). F-261,
  F-263 and F-264 change a verdict; F-265 is the core's.
- F-268 to F-272 were found against `arbitration.v1`'s text 0.1, the
  eleventh profile, and core 0.54, read cold and run live, zk2py against
  itself, in round 77 (`SPEC-FINDINGS.md`, "At arbitration.v1 0.1 and core
  0.54"). F-268 changes a verdict; F-269, F-271 and F-272 a test; F-270 is
  a test missing.
- `timing.v1` 0.2 and core 0.55 resolved F-259 to F-267, and
  `arbitration.v1` 0.2 and core 0.56 F-268 to F-272 (round 78, against
  the Rust runtime). F-273 to F-276, found there, went with both texts'
  0.3 and the reference's black boxes; F-277, found in round 79 between
  the two tools, with `timing.v1` 0.4.
- F-278 to F-280 were found in round 80, the final cold read of
  `timing.v1` 0.4 and `arbitration.v1` 0.3, by asking both tools about
  windows the scenarios do not make (`SPEC-FINDINGS.md`, "At timing.v1
  0.4"). All three are `timing.v1` §2.7's and §5's, a tool's: F-278 and
  F-280 change a verdict, F-279 is wording. `arbitration.v1` 0.3 has none
  open.
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
  - `live.check_s4` reads §4.2's two selectors, `@/*/router` and, since
    0.48, `@/*/router/status/plugins/storage_manager/storages/**` (0.11's
    `@/*/router/**/storage_manager/storages/**` until round 73), with
    `plugins` beside them.
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
- **the Tool's admin read** (0.15, 0.50): `query` on `@/*/router`,
  `@/*/router/status/plugins/storage_manager/storages/**` and `@/*/*`, and
  their `reply`. 0.15's `@/*/router` and `@/*/router/**` are compiled only
  for security.md §4 step 4 (`admin_read=acl.ADMIN_READ_015`,
  `hazard_ok=True`);
- **no allowed ingress query reaches a node's declaration records**
  (§11.1, 0.50; under `deny`, on an ingress `query`, 0.51): `generate`
  refuses a block whose allow rules would (`HazardousGrant`), and
  `acl.block_hazards(block)` finds one in any block (see "The admin read
  under access control");
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
ending `…/storage_manager/storages/<name>` (§4.2, 0.15). Under 0.11's
selector a router's `router/queryable/<…/state/**>` records answered it;
under 0.48's none does (see "A router's admin space beside a router that
is declaring").

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
  instances whose first answer is yes (0.2), and not a recognised standby
  (0.6, `redundancy.v1` §2.15, over reads of the address's `@zk/**`:
  F-208). It reads their contracts' `uses` by the fingerprints their
  descriptors name.
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
  (0.27), 15 of 15. Since round 73 it runs §3 and §12 too (core 0.48,
  below), 22 of 22 in all. §11's W is a peer session without an HLC, which puts
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

## alarms.v1 (profile text 0.8, core 0.34)

```bash
just py-alarms                                   # against zk2py's owner
just py-alarms --owner <exe>                     # against the Rust owner example (or ZK2PY_OWNER)
just py-alarms --only 2 --full                   # §2 at the contract's 900 s, about 8 minutes
```

`alarms.v1` is the sixth profile (`spec/profiles/alarms/`). zk2py read its
0.1 cold in round 38, its 0.2 cold and live in round 39, its 0.3 in round
40, its 0.4 and 0.5 in round 41, its 0.6 in round 42, its 0.7 in round 43,
and its 0.8 in round 44. Its standard
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

## telemetry.v1 (profile text 0.5, core 0.35)

```bash
just py-telemetry                                # §1–§9 against zk2py's owner, about a minute
just py-telemetry --owner <exe>                  # against the Rust owner example (or ZK2PY_OWNER)
just py-telemetry --tool <zenctl>                # zk2py's owner, zenctl telemetry judging too (or ZK2PY_ZENCTL)
just py-telemetry --only 9                       # one section
just py-telemetry --only opening --tool <zenctl> # the tool's window opening alone
```

`telemetry.v1` is the seventh profile (`spec/profiles/telemetry/`), and the
first whose first point is a standard schema (core 0.35), not a standard
contract. zk2py read its 0.1 cold in round 45, its 0.2 cold and live
against the Rust owner example in round 46, its 0.3 cold, with the Rust
tool `zenctl telemetry` judging beside zk2py's reader, in round 47, and its
0.4 cold, with the fixed tool, and through once more as a whole, in round
48, and its 0.5, the closing round, in round 49. Rounds 50 and 51
confirmed the tool after TG and TH.
- **`zk2py.telemetry`**, the reader's and the tool's half, from the text
  alone:
  - **the standard schema** (§2.3, §3): the two published ids, oldest
    first, and `is_profile_type`, a type the profile's by its kind, its
    name and its artifact's id, and only from the revision that defines it;
  - **the declaration** (§2.1, §2.2, §2.4): `declare`, every reason in
    §2.2's order, each key alone and a check that needs a wrong value
    skipped; `carries`, §2.4's table over the root's `type` as a set, so
    both spellings of a nullable carry nothing; `contract_declarations` and
    `bundle_declarations` find each type's root with its `$ref`s followed,
    as core §9.8 follows them;
  - **a sample's bytes** (§2.6, 0.2, 0.3): `decode_json`, strict JSON with
    every number finite by rounding, an integer's text read exactly;
    `decode_cbor`, a bignum (tags 2 and 3) its integer and any other tag its
    content, a chain of tags read without recursion
    (`zk2py.cbor.loads(…, bignums=True)`); both refuse a data item nested
    past 256 arrays and maps (`depth`); `decode_attachment`, an Epoch
    attachment by its own encoding;
  - **a sample** (§2.5, §2.6): `conform`, the twelve reasons in order, the
    member's kind and unit as the sample gives them; an Epoch attachment
    that does not decode is `epoch_invalid`, and a member written `null`
    is present;
  - **a pair** (§2.9 to §2.11): `epoch_of`, `continuity` and `rate_of`,
    `double(d) / (double(e) / 1e9)` in Python's IEEE doubles, equal bit for
    bit to every fixture rate; `ntp64_ns`, a stamp's nanoseconds as §2.11
    states them; `kept_reads`, a state member's samples through core §4.2's
    merge;
  - **a window** (§5's "over many pairs", 0.3, 0.4): `member_answer`, one
    member's continuity answer, and `verdict`, a finding, no, or could not
    tell. It could not tell when any counter or histogram read formed no
    pair, or only assumed ones, whatever another formed (0.4), and when any
    sample read was undecodable or under an invalid declaration (0.5).
- **The owners**, behind one adapter in `zk2py.telemetry_scenarios`:
  - **`PyMeter`**, over zk2py's owner: a checked writer that fills the
    epoch where a put leaves it out, and refuses what its declaration
    refuses and a decrease within one epoch;
  - **`RustMeter`**, the owner example with `--telemetry`, a black box
    driven from its `--help` alone: `telemetry put|raw <resource> <json>
    [<attachment json>]`, `telemetry new-epoch`, `telemetry instance`, and
    its refusals read by their `<why>`. It is given no contract file, and
    §1 checks that its embedded `meter.v1` fingerprints as zk2py's
    transcription of the text does.

  A raw step is offered to the checked writer first, its refusal recorded,
  and then sent unchecked. The example's refusals are read only in the form
  its help gives (`refused <line>: telemetry.v1 <why>: <words>`, the
  problems §2.6's tokens); any other answer fails its section with the
  answer as it came, and every check that reads a reason shows the raw
  answers. The driver writes each Point's `timestamp` from
  the host's clock.
- **The `telemetry` conformance family:** 269 of 269. 0.1's cases passed
  the first time they ran, and 0.2's once zk2py followed F-143's and
  F-144's answers.
- **The vocabulary** (core 0.35): `telemetry`'s four keys moved from zk2py's
  interim table to its published one, and W105 reads them as before.
- **`just py-telemetry`** (`python -m zk2py.telemetry_scenarios`) runs
  `scenarios.md` §1 to §9, each on an in-process router of its own,
  against `meter.v1`, transcribed from the text into
  `interop/telemetry/`.
  - R and R0 are client sessions of R1. R's liveliness watch and both
    subscriptions are declared before the owner starts.
  - §8 runs twice: R0 alone, as the reference runtime does, and R0F behind
    a second router whose access control denies `zk2/*/*/@zk/**` on its
    `unixsock-stream` face. A control off the face sees the tokens R0F
    does not.
  - §9's R is a late joiner of `total`, holding its GET's reply unread
    until 12 has arrived, beside a witness that shows the put's stamp.
  - R's presence view stands on a complete liveliness read after its
    subscription (§2.9, 0.3), and gives an arrival before it none.
  - **With `--tool <zenctl>`** (or `ZK2PY_ZENCTL`), `zenctl telemetry`
    judges each section beside zk2py's reader, a black box read from its
    `--help`. It starts after the owner is up, and the steps start the
    moment its "listening" line appears (F-154's answer; round 47 needed
    1 s with `zenctl-tc`), across a face too since TG (F-159's answer). In
    §8 it stands across the real face with
    `--across-face`, and in §9 its own GET seeds 10. Member by member, it
    is compared with R, with R0F in §8, and with §9's witness: kind and
    unit, the samples' counts and problems, each pair's continuity,
    epoch, evidence, time base and rate (exactly, unless timed by
    receipt), each member's `continuity.answer` and `pairs_listed`, and its
    exit against `verdict`. Ten windows are built for §5's SHOULDs, three of
    them for 0.5's over many samples with `meterx.v1`, zk2py's own
    interface whose `stream/odd` is declared invalid, and two probes check
    the tool's answers to F-154 and F-155. `tool_opening` (`--only
    opening`) asserts F-159's and F-160's answers: 20 tries each across the
    real face on S4's GET (`total`'s value checked held first), across it
    on `--settle`, and with no face. Each sample put the moment "listening"
    appears is read. Each line says positively which opening it stood on,
    in the help's words. Each exit is zk2py's verdict over a view that
    read the sample. `--only opening` without `--tool` exits 2. Each window
    also checks that zk2py's reader read samples in it and that the tool
    read every one of them, so the mode cannot pass testing nothing. A run
    logs the tool's `--version`; here `zenctl 0.12.0
    (0.14.0-794-g905b3684-dirty)`, the `zenctl-th` build.
  - The exit is 0 when every check passes, 1 when one fails.

  | § | zk2py's owner | The Rust owner example (`owner-tb`) | zk2py's owner, the tool judging too (`zenctl-th`) |
  |---|---|---|---|
  | 1 A counter across a restart (+ the fingerprint, a new session's stamps) | 12 of 12 | 12 of 12 | 18 of 18 |
  | 2 A counter across `new_epoch` in one process | 6 of 6 | 6 of 6 | 12 of 12 |
  | 3 A decrease within one epoch | 6 of 6 | 6 of 6 | 12 of 12 |
  | 4 A Point whose tag disagrees | 5 of 5 | 5 of 5 | 11 of 11 |
  | 5 Histograms, other bounds and a decrease | 5 of 5 | 5 of 5 | 10 of 10 |
  | 6 Absence is not zero | 3 of 3 | 3 of 3 | 10 of 10 |
  | 7 An in-band epoch on a bare counter (+ the checked writer's refusal of step 3; the undecodable attachments, zk2py's owner only) | 6 of 6 | 5 of 5 | 11 of 11 |
  | 8 A counter that crosses a face (R0, and R0F across a real face) | 8 of 8 | 8 of 8 | 14 of 14 |
  | 9 A state counter seeded (0.2) | 5 of 5 | 5 of 5 | 10 of 10 |
  | §5 over many pairs and samples (0.3 to 0.5): ten windows built for the tool, and F-154's and F-155's probes | | | 42 of 42 |
  | The tool's opening (F-159's and F-160's answers, TG and TH): three openings, 20 tries each | | | 13 of 13 |
  | §2.11's conversion against zenoh-python's `as_nanos` | 1 of 1 | 1 of 1 | 1 of 1 |
  | **Total** | **57 of 57** | **56 of 56** | **164 of 164** |

  - **The tool agrees with zk2py's reader on every member of every
    section,** its exit and each member's answer included. §8 across the
    face exits 2 since 0.4: `drops` forms only assumed pairs, and `frames`'
    in-band epoch answers nothing of it. 0.5's windows exit 2 beside an
    undecodable sample and a member under an invalid declaration, and 0 with
    the invalid resource unheard, in both readers.
  - **The example's raw** takes an attachment's JSON only. §7's zero-byte
    and `x` attachments therefore run against zk2py's owner alone.

## desired.v1 (profile text 0.8, core 0.39)

`desired.v1` is the eighth profile (`spec/profiles/desired/`), and the
second whose first point is a standard schema, `desired.v1.json`, typing a
target's `desired.v1.Report`. zk2py read its 0.1 cold in round 52, with
core 0.36, which publishes its vocabulary (`target_param` alone, so
`desired.target` draws W105, `contract.py`), and its 0.2, the answers to
round 52's twenty findings, with `config.v1` 0.4, in round 53.
- **`zk2py.desired`**, from the text alone:
  - **the standard schema** (§3): the two published ids, 0.1's and 0.2's,
    and `is_profile_type`, a report found by type identity, never by name;
  - **the declaration** (§2.2): `declare(kind, template, annotations,
    params)`, the seven reasons in order, each alone, the value's checks
    skipped when the value is wrong, `not_string` read only of a name that
    is a single parameter (0.2); one target parameter binds `either`
    identity, two bind `self.system` then `self.service`; a horizon,
    whatever its value, invalidates nothing;
  - **the binding** (§2.3): `bind`, zk2py's own `target_address` for a
    target that is not `<system>/<service>`, then the five findings in
    order (`wrong_self` a parameter bound to the other's `self`), the kill
    switch, and `slice_key`, each target parameter its identity slugged
    (core §1.4), under `@state` for an explicit resource;
  - **versions** (§2.4, §2.6): `Stamp`, written and read in zenoh's form,
    its id never 0, compared by value;
  - **the target** (§2.6 to §2.10): `TargetRole`, a pure state machine.
    `receive` judges against the version last acted on or adopted (the
    running act's while one runs), then every id it remembers (0.2: per
    id the latest time acted on or adopted, for the 16 most recent ids,
    `REMEMBERED_IDS`), then the one held waiting. It answers `unstamped`,
    `same_version`, `confirmed` (the author's sample of a version held,
    or held waiting, from an archive), `older`, `held`, `adopted` (an
    admitted version whose document equals the last act's or adoption's,
    a delete equal to a delete), `act` or `revert`; `finish` takes
    `done`, `refused` or `failed` (a refused revert read as failed) and
    starts the held version; `call` and `restart` (which forgets every
    id); and the Report after each step, with how many times it was put;
  - **equal documents** (§2.7): `same_document`, for a payload encoded as
    JSON that reads as strict JSON (finite by rounding, at most 256
    deep), structural equality with every number by its exact value (0.2):
    its text read with every non-integer number as an `Exact`, a
    normalised (sign, digits, exponent) form computed from its text alone,
    so the cost is the text's length, never the exponent's value
    (`strict_json(…, exact=True)`; an exponent past 25 digits kept as
    text), a decoded float as its double's exact digits; byte for byte
    otherwise. A value strict JSON cannot yield equals nothing;
  - **reading a report** (§2.10): `read_report_bytes` and `read_report`,
    strict JSON (RFC 8259's whitespace, no duplicate member, every number
    finite by rounding, a UTF-8 form, at most 256 deep), a non-integer
    number read as its double (F-181), the schema, the stamps' form, the
    revisions' range, then the eight consistency findings in order;
  - **§2.17's conclusions**: `conclude`, §5's four questions over one
    reading, each `yes`, `no`, `not_yet`, `unobservable` or `not_asked`
    with its reason; across a face, the deployment's word answers step 2
    (0.2), and no word is `no_descriptor`.
- **The fixtures at 0.2:** 341 checks over 258 cases, every one passing:
  `schema.json` 3 cases and five checks of its header and zk2py's tables;
  `declarations.json` 43; `bindings.json` 25; `order.json` 78, each case's
  report also read back with no finding (78 more); `reports.json` 74;
  `conclusions.json` 35. Round 52's code stopped at `order.json`'s new
  `text` input, and its readings of F-163, F-166, F-168 and F-170 would
  have failed the cases 0.2 added for them.
- **Passing is not deriving.** Each 0.2 rule was implemented from the
  text first. `order.json` case 72 fails with 15 ids remembered, so the
  memory's floor is pinned; its ceiling is not, as the text says. No case
  tells F-181's or F-184's readings apart.
- **0.3 (round 55):** a report's numbers, and a document's revision, are
  read by their exact values (`exact_json`, `revision_of`).
- **0.4 (round 57):**
  - a **run** (§0) is the unit of a target's memory, as `TargetRole`
    already kept it: `restart` begins one, a re-mint keeps it;
  - **where a refusal points** (§2.7, F-193): `refusal_pointer`, a
    member's own schema at the member, a required member missing at the
    object holding it, a value of the wrong type at itself. `order.json`'s
    `{"finish": "check"}` judges the running act's put by the file's own
    `schema` (`check_finish`), and its eight cases pass, with the live
    targets' pointers from the same function.
- **0.5 (round 59):** a tool's reading before §2.17 answers, session-free:
  - `take_slice`: the revision the author's descriptor names, in hand;
    else a revision of its interface held offline that declares a
    desired resource the requirement names (F-213), the latest minor
    first; else the target alone, each template its requirement names
    under `state` then `@state` a candidate (F-210). With the named
    revision in hand and no desired resource of it named, the role is
    not this profile's (F-217);
  - `which_report`: a candidate's readable claim, two equal being one,
    two differing unreadable (F-212), else an unreadable reply, else none,
    or no slice;
  - `slices.json`'s 22 cases pass on them, and the tool reads through
    them. A version's age on a trusted clock is taken at its GET's answer.
- **0.6 (round 61):**
  - `bind` takes `also`, the other desired resources the requirement
    names. Any is `two_documents`, after `required_unbound` and the kill
    switch, and alone (F-218);
  - `take_slice` reads every desired resource the revision in hand names.
    None is no target's role, `not_a_target`, whether the revision is
    named or held. Two or more is `two_documents`, no slice. The target
    alone is read only with no revision in hand (F-217). `find_report`
    answers `not_a_target` for such a role;
  - the target (`desired_live.Target`) reads its role's requirement
    against its author's declaration at its start. A role naming two or
    more desired resources refuses the start, `not started: … two_documents`,
    nothing declared (§2.1). Its driver takes a `tasking.v1` or
    `worker.v1` file in place of the scenarios', as the owner example
    does.
- **0.7 (round 62):** the start's refusal is scenarios §11, which the
  runner now runs as written, S watching the instance tokens by
  liveliness and `Rp`. A requirement naming no resource names every one,
  and so does a manifest's role, which names no declaring contract
  (core §3.1, §3.3). zk2py read both so already; its tool now says
  `requirement_unread` only when a declaring contract is named.
- **0.8 (round 64):** a requirement whose declaring contract the
  descriptor names and the tool does not hold, or holds in a revision
  declaring no such role, is unread (F-225). `take_slice` gives no slice
  and no binding problem: `requirement_unread`, answered unobservable,
  `no_report`. A revision in hand declaring no desired resource at all
  still gives `not_a_target`. The tool's row names the declaring
  contract, `requirement_unread`, beside a kill switch too, as the
  reference's does.

**The live half (round 55).**

```bash
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.desired_scenarios \
  [--author zk2py|<owner-example>] [--target zk2py|<owner-example>] [--owner <owner-example>] \
  [--only N]... [--poll] [--zenctl <zenctl> | --no-zenctl]
```

The Rust tool has no default: `--zenctl <zenctl>` or `ZK2PY_ZENCTL`.
Without either, §9 and the shapes fail, unless `--no-zenctl` runs
zk2py's tool alone, its other checks counted as not run. Each party is
zk2py's unless a path to the owner example is given.

- **`zk2py.desired_live`.**
  - **The author.** It owns `tasking.v1`'s `tasks/{system}/{service}`,
    writing a document only when its content changes, deterministically,
    every number by its exact value (`desired.canonical_json`). Its record
    is the owner example's format. Before its tokens it makes each delete
    within its window again, for what is left of the window, and puts each
    document again, its clock caught up past the record's stamps. Each
    entry stands alone: a delete with no readable `at_ms` cannot be placed
    in its window, so it is dropped and logged, never dated now; a date
    after now counts from now and is kept so; a write that fails is logged
    and carried in the record; none stops the tokens. A re-mint in its own
    session does the same, and drops a delete whose window has passed.
  - **The target.** It owns `worker.v1`, its role bound with
    `self.system` and `self.service`, or unbound. It subscribes, then
    seeds, and follows its author on a watch session or reads its presence
    every 500 ms. It seeds at each appearance, and up to four times more
    while a seed draws nothing. Seeds coalesce: one runs, at most one
    waits, and a request while one waits joins it. It reads an archive only after a complete
    read shows its author absent. `TargetRole` judges each version. The
    scenarios' drive acts, with hold, fail-next and refuse-next. Its report
    goes out before its tokens and after each change, and a re-mint keeps
    its acts.
- **`zk2py.desired_owner`** is either party as a process, with the owner
  example's `--desired-*` options and `desired …` commands, `desired seeds`
  and `desired gets <target>` among them since 0.4, in the example's words,
  plus `desired queries`.
- **`zk2py.desired_tool`** (round 57) is §2.17's tool on a bus. For each
  role it reads, on a session that has never held a liveliness subscriber:
  - the target's presence and descriptor;
  - the role and its binding;
  - the slice (`desired.take_slice`, 0.5), from the author's contract
    (the revision its descriptor names, held offline or retrieved), else
    a revision of its interface held offline (F-213), else the target
    alone: each template its requirement names, under either kind token
    (F-210);
  - the report, among every report resource's replies, chosen as §2.10
    says (`desired.which_report`), aged only on a trusted clock;
  - the author's presence, and its current version by S4's GET. An
    unstamped reply is `author_read_incomplete`, and silence
    `author_silent`. An absent author's archived last-known is read
    beside, never current, from every archive's `archive.v1` token.

  `watch` reads every 500 ms over a window, and finds a target behind
  only when it stays not yet on one version for the whole window, or on
  a trusted clock, the version's age taken at its GET's answer (0.5).
- **`zk2py.desired_scenarios`** runs `scenarios.md` §1 to §10 with any
  two parties as black boxes. Its bus has R1, R2 behind a link it cuts,
  S, S2, G, X and a stand-in archive. §10 includes steps 3 and 4 (0.4),
  and §9 is the tool's section: zk2py's tool and the Rust one,
  `zenctl desired`, the doctor's `desired-*` checks and
  `check conform`'s `desired-*` cases, as black boxes
  (`--zenctl` or `ZK2PY_ZENCTL`). It adds zk2py's edge documents, a target's re-mint,
  and twenty-three §2.17 shapes no scenario makes, built by test sessions
  and asked of both tools.
- **The scenarios' contracts**, transcribed from `scenarios.md` into
  `interop/desired/`, give the owner example's fingerprints, beside
  zk2py's own:
  - `zk2py_worker2.v1`: a second report resource, so that two reports can
    claim one slice;
  - `zk2py_worker3.v1` (0.5): a role naming two templates, read from the
    target alone as four candidates (F-210);
  - `interop/desired-explicit/tasking.v1`: the author's interface with
    its tasks explicit, held in place of `interop/desired`'s (F-213);
  - `zk2py_watcher.v1`, a role on no desired resource (F-217);
    `zk2py_tasking2.v1` and `zk2py_worker4.v1`, a role naming two desired
    resources (F-218); and `zk2py_workerall.v1`, a role naming every
    resource;
  - `interop/desired-two/` (0.6): `tasking.v1` with two desired
    resources, and `worker.v1` whose role names both, which a target
    refuses at its start (F-218).

Results at 0.4 (round 57): every check passes in all eight runs. The
runs are four pairs of parties, each with targets on a watch session and
polling, against the owner example built from PR #856's head
(`<owner-example>`), with the reference's zenctl (`<zenctl>`) as §9's
Rust tool:

| Author → target | Watch | Polling |
|---|---|---|
| zk2py → zk2py | 105 of 105 | 83 of 83 |
| Rust → Rust | 83 of 83 | 83 of 83 |
| zk2py → Rust | 83 of 83 | 83 of 83 |
| Rust → zk2py | 83 of 83 | 83 of 83 |

Each run counts:
- §1 to §8 and §10: 43 checks, §8's `desired gets` and §10's steps 3 and
  4 included;
- §9: 29, each step asked of both tools, with the doctor's and
  `check conform`'s rows;
- edges: 9, every refusal, pointer and exact revision agreeing between
  the two targets;
- re-mint: 2.

The watch run of zk2py → zk2py adds the tool's 22 shape checks: eleven
shapes, each asked of both tools. Both tools answer every §9 step and
every shape alike, and as the text does. §1's target converges 1.11 s
after the heal. The owner example's author now puts each number by its
exact value (F-194). §2's X builds every timestamp from the one S
received with `s2` (F-196). Round 55 at 0.3: 52 or 51 checks a run, in
the same eight runs.

Results at 0.5 (round 59): §9 and the shapes rerun with both tools, the
owner example and zenctl built from main after PR #859
(`<owner-example>`, `<zenctl>`). Every check passes:
- §9, 29 of 29 in each of the eight runs: the four pairs of parties,
  targets on a watch session and polling;
- the shapes, 69 of 69: twenty-three, each asked of both tools, and
  the two tools' rows compared member by member (listing, slice and
  where it came from, the author's presence and GET, the report found,
  the binding's and declaration's problems).
  - Five are 0.5's rules. Four candidates from two templates, the report
    choosing. None claimed, `no_report`. Two claiming two candidates and
    differing, `report_invalid` (F-210). A held revision declaring the
    resource explicit: a report under `state` is none, and one under
    `@state` is the report (F-213).
  - Seven are shapes the text does not state. A role on no desired
    resource, two ways (F-217). A role naming two desired resources,
    three ways (F-218). A role naming every resource, without and with
    the author's interface held.

The two tools agree on every §9 step and every shape, the seven the text
does not state included. Their rows differ in one member, on the two
shapes where two reports claim the slice and differ. There zenctl's row
carries no report, and zk2py's says `unreadable`, as §2.10 does. Both
answer `report_invalid`.

Results at 0.6 (round 61): the owner example and zenctl built from PR
#864's head. Every check passes:
- §9, 29 of 29 in each of the eight runs;
- the shapes, 69 of 69. 0.6 moves three expectations, to `not_a_target`
  for a held revision's plain resource and to `two_documents` twice (no
  slice, `no_report`). The two rows now agree member by member on every
  shape, the reference's row saying `unreadable` of two reports that
  differ;
- `two-documents` (zk2py's own, §2.1; scenarios §11 since round 62), 2 of 2 in each of the four
  pairings. Each target, zk2py's and the owner example, given
  `desired-two`'s contracts, refuses its start: exit 1, no instance
  token, `two_documents` said. Each starts with only the worker given, its
  author's declaration holding one of the two;
- §1 to §8, §10, the edges and the re-mint, zk2py's parties, 54 of 54
  after the target's change.

Results at 0.7 (round 62): the owner example and zenctl built from main
8025fd99. Every check passes:
- §9, 29 of 29 in each of the eight runs;
- §11, 2 of 2 for each target. Given the two copies, zk2py's target and
  the owner example each stop with exit 1 and `not started: …
  two_documents`. S sees no instance token within 5 s, and no report.
  The control, naming one of the two, starts, its report `{key: D,
  source: baseline}`. The owner example's message now carries its
  `not started:`;
- the shapes, 75 of 75: twenty-five, two new for a manifest's role, on an
  author with two desired resources (`two_documents`) and with one (its
  slice). The rows agree member by member on all.

Results at 0.8 (round 64): `owner-dgg` and `zenctl-dgg`. Every check
passes:
- §9, 29 of 29 in each of the eight runs;
- §11, 2 of 2 for each target;
- the shapes, 90 of 90: thirty, five new.
  - F-225's measured shape, `zk2py_worker9.v1` at a fingerprint no one
    serves, on an author with two desired resources: `no_report`,
    `requirement_unread`, no `two_documents`.
  - The decided shapes: one desired resource, unread too; a revision
    with no desired resource, `not_a_target`; a declaring contract in
    hand without the role, unread; unread and unbound, the kill switch.

  The rows agree member by member on all thirty, `requirement_unread`
  included.

Asked beyond the shapes, `zenctl check conform`'s `desired-binding` finds
a two-documents role, `two_documents`. The doctor, which holds no
contract offline, reads it unobservable over the shapes' test sessions,
which serve no bundle.

## redundancy.v1 (profile text 0.5, core 0.40)

```bash
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.redundancy_scenarios \
  [--a zk2py|<owner-example>] [--b zk2py|<owner-example>] [--owner <owner-example>] \
  [--only 1 … 12|remint|poll|poll-miss|heal-retry|heal-retry-pause|remint-campaign]... [--runs 20] \
  [--zenctl <zenctl>]
```

§12's Rust tool comes from `--zenctl <zenctl>` or `ZK2PY_ZENCTL`, and has
no default. Without either, §12 runs zk2py's tool alone and says so.

`redundancy.v1` lets several instances of one service run at once: one
serves a governed interface, and the others stand by. Core 0.39 marks a
standby in the descriptor (`"standby": true`), with D012. zk2py read text
0.1 cold in round 56, with core 0.38 and 0.39 and `hostid.v1` 0.6. It
read text 0.2 in round 58, with core 0.40 and `hostid.v1` 0.7, and ran
it against the Rust runtime for the first time. It read text 0.3 in round
60, and ran §12 against the Rust tools for the first time. It built each
from the text:
- **`zk2py.redundancy`**, session-free:
  - the declaration (§2.2: governed, its election, or the reasons in
    order);
  - an entry's mark (§2.4, §2.14);
  - one read's election (§2.5, §2.11) and the heal (§2.9);
  - `Machine`, the state machine of `lifecycle.json`. Since 0.2 it keeps
    what is heard while a read or an act runs, and decides it there
    (F-199). A heal read that ends nothing is made again (`retry`), and
    its own re-mint is `{"reminted": id}` (F-209);
  - `poll_read`, §2.7's poll (0.3, F-215). Each read is compared with
    the last's, the first with no holder, and a read not complete
    removes no holder. A holder new to it is an appearance, and each one
    a complete read shows gone is a withdrawal;
  - §5's four questions (§2.15).

  180 of 180 fixture cases. 0.2's thirteen `lifecycle.json` cases and
  0.3's seven `polls.json` cases passed the first time they ran.
- **The owner** (`zk2py.owner`) stands by on an interface:
  - marked in its descriptor from the start, validated by step 2, with
    nothing declared;
  - `take_over` does core §8.2's steps 1 to 4 for it;
  - `step_down` does their reverse, and forgets what it held of the
    interface (§2.6, 0.2);
  - a standby's puts and deletes are refused, saying so (§2.4, 0.2);
  - `lease_ms` sets its session's lease.

  At start it refuses an invalid declaration, a governed interface in the
  tokenless set among them, and a standby on a tokenless interface (D012).
- **The checks of the core.** D012 is cascade 7 in `descriptor`.
  `redundancy` is in the published vocabulary (W105). The fault shapes
  read the mark: a standby is no "no token", and a holder marked standby
  is `standby holding`.
- **`hostid.v1` 0.6 and 0.7** (`live.hostid_collision`): a recognised
  standby is not counted, and since 0.3 it is named beside the answer.
  The finding's two reads are of the address's `@zk/**` (F-208, 0.7).
  `hostid_scenarios` §6 step 6 passes.
- **`zk2py.redundancy_live`:**
  - `Candidate` drives `Machine` from one worker thread, on three
    sessions: the claim's, a watch on `zk2/<address>/@zk/**` with its
    round trip, and a reader, after a round trip on the claim's session
    (F-204). What its watch hears while a read or an act runs reaches the
    machine before that read's answer or act's finish, which keeps it.
    Its re-mint is told to the machine first. With `poll_s` it polls
    instead of watching (§2.7, 0.2). Under `external`, the lock is a
    `flock` tried every 20 ms, or the commands' lock.
  - `Follower` is §2.10's consumer, following the lowest holder's
    `meta.zid`.
  - `read_tool` gives §5 from two reads a grace apart (§2.15, 0.3):
    - governance from a contract held offline, or retrieved;
    - an invalid declaration read `no_contract`, its reasons beside;
    - with `with_probe`, the probe between the two reads. It is a GET,
      `All` and `None`, over each fan-out-forbidden operation's wildcard:
      `*` for each parameter, or `$*` after a template-less name. Each
      reply is attributed by zenoh's replier zid. A reply from an
      instance marked standby with no token in both reads is the
      finding, beside §5's answers. Every revision the descriptors name
      is probed, each key once. A wildcard that would reach an operation
      whose fan-out is allowed, in any revision in hand, is not probed
      (F-221). A revision not in hand, or a failed probe, goes in
      `not_probed` with why.
  - `split_findings` is core §6's split-brain with "one split, one
    finding" (0.3): a governed interface's split by its first question,
    whatever the marks, and core §6's rule only where the profile does
    not govern.
- **`zk2py.redundancy_owner`** is one candidate as a process. Since round
  58 it takes the owner example's command line (`--standby`,
  `--instance-id`, `--retry-ms`, `--grace-ms`, `--poll-ms`, `--lease-ms`,
  `--lock-file`, `--refuse-without-lock`, `--hold-heal`, `--peer`). It
  answers the owner example's `redundancy role|reads|lock|calls|
  hold-heal|wait|new-epoch|zid` and `put <iface> <resource>` in its
  JSON. Since 0.3 that includes `--wait-go`, which opens every session,
  says `waiting go` and starts at `go`; `redundancy zid`, its three
  sessions' zids; and the poll's `poll_log` (F-216). It also takes
  `--hostid-root`, for a minted address. zk2py's own extras are
  `redundancy steps|log` and `--reader-connect`.
- **`zk2py.redundancy_scenarios`** runs `scenarios.md` §1 to §12 with
  any two candidates, zk2py's or the owner example (`--owner
  <owner-example>`), as black boxes, both driven by the same commands.
  It also runs six sections of zk2py's own:
  - `remint`: an incumbent's re-mint;
  - `poll`: §2.7's poll, and what it misses;
  - `poll-miss`: a polling standby whose polls are not complete while
    its incumbent goes (F-215);
  - `heal-retry`: a heal whose reader is cut apart from its watch;
  - `heal-retry-pause`: a heal read across a paused link, for either B;
  - `remint-campaign`: a re-mint while campaigning.

  `PauseLink` is a relay that stops reading either way and resumes,
  nothing lost, shorter than either lease.

  A crash is a SIGKILL. A lost host is a SIGKILL behind `SilentLink`,
  with a 2 s lease (F-201). The bus reads what the owner example's
  standard input does not show: a put by its stamp (its `put` writes
  `ok`), and a step-down from its tokens.

  §3 (0.3, F-216) holds both candidates at their own `--wait-go` and
  sends both `go`. Each run's start is measured at S, by its two instance
  tokens' arrival. A run more than 10 ms apart is made again, and counted
  apart. Since 0.4 the owner example's `--wait-go` starts at `go`
  (F-220), and `--rust-spawn-at-go` is gone. §1's and §3's "before the
  token" reads an interface token at S within the GET's own round trip of
  its unmarked answer as the takeover's (0.4, F-223).

  §12 asks zk2py's tool and zenctl's `redundancy`, `doctor` and `check
  conform` of each shape, both holding `ledger.v1` offline, and compares
  their answers.

**Results at 0.5 (round 65):** every check passes in all four pairings,
against `owner-re` and `zenctl-re`, with the same counts as 0.4's: 105,
102, 104 and 103. §12 is 33 in each, step 9 in the place of round 63's
shape beyond step 8. In step 9, A and B run `ledger.v1`, and T also holds
`ledger-appendall` offline:
- neither tool asks `append$*`;
- each names `appendall` and the revision held offline, by its
  fingerprint;
- each reads the standbys' exposure unobservable, `check conform`'s
  `redundancy-standby` with exit 2.

zk2py's tool now says the exposure (`ToolReading.exposure`: a finding,
none, or unobservable when no operation was probed). §3's median skew
was 0.6, 0.5, 0.6 and 0.9 ms, none made again. The two tools agree on
every shape in every pairing.

**Results at 0.4 (round 63):** every check passes in all four pairings.
The Rust side is `owner-rd` and `zenctl-rd`. §3 ran 20 measured runs in
each, both candidates at their own `--wait-go`.

| Pairing (A, B) | §1–§12 | of it §12 | zk2py's own | All |
|---|---|---|---|---|
| zk2py, zk2py | 90 | 33 | 15 | 105 |
| Rust, Rust | 89 | 33 | 13 | 102 |
| zk2py, Rust | 90 | 33 | 14 | 104 |
| Rust, zk2py | 89 | 33 | 14 | 103 |

- **§3:** the tokens' median skew at S 0.6, 0.4, 0.7 and 1.1 ms, at most
  3.9 ms. No run was made again. B kept serving in 1, 0, 0 and 8 runs,
  and the heal ended it in 2, 0, 0 and 2.
- **§12 step 8's `ledger-appendall` half**, both tools. Neither asks
  `append$*` nor `appendall`, and each says why. `check conform`'s
  `redundancy-standby` is unobservable, exit 2. `appendall`'s handler
  runs 0 times, counted by each owner's executions.
- **Beyond step 8**, both tools on `ledger.v1`, with `ledger-appendall`
  held offline and named by no descriptor: neither asks `append$*`
  (F-226).
- **F-222's shape:** `check conform` with every case skipped asks no
  case and runs no handler.

**Results at 0.3 (round 60):** every check passes in all four pairings.
The Rust side is the owner example and zenctl built from PR #862's head
(`owner-rc` and `zenctl-rc`), its owner spawned at `go` for §3. §3 ran
20 measured runs in each.

| Pairing (A, B) | §1–§12 | §1 | §2 | §3 | §4 | §5 | §6 | §7 | §8 | §9 | §10 | §11 | §12 | zk2py's own | All |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| zk2py, zk2py | 87 | 11 | 3 | 1 | 2 | 6 | 3 | 7 | 3 | 4 | 11 | 6 | 30 | 15 | 102 |
| Rust, Rust | 86 | 11 | 3 | 1 | 2 | 5 | 3 | 7 | 3 | 4 | 11 | 6 | 30 | 13 | 99 |
| zk2py, Rust | 87 | 11 | 3 | 1 | 2 | 6 | 3 | 7 | 3 | 4 | 11 | 6 | 30 | 14 | 101 |
| Rust, zk2py | 86 | 11 | 3 | 1 | 2 | 5 | 3 | 7 | 3 | 4 | 11 | 6 | 30 | 14 | 100 |

- **§11** now checks the reason in every pairing. The owner example says
  `not started: … invalid: value` (F-214).
- **§3, measured at S.** The two instance tokens' median skew was 0.2 ms
  (zk2py, zk2py), 1.2 ms (Rust, Rust), 3.7 ms and 2.5 ms (the two mixed).
  Runs made again, more than 10 ms apart: 0, 5, 1 and 1. A run whose
  token S never saw is counted apart again, and none was. In the 20 runs
  B kept serving in 2, 2, 6 and 6, and the heal ended it in 2, 1, 1 and
  0.
- **§12, both tools, on every shape:** steps 1 to 8, with zenctl's exit
  codes. Beyond the text, they agree on two more shapes:
  - an invalid declaration on the bus: `no_contract`, `value` beside;
  - a probe that would reach an allowed operation: not probed (F-221).

  For step 6, both name B a recognised standby. For step 7, both report
  step 2's split once, as this profile's. For step 8, both find X's
  queryable, attributed by its session's zid. The doctor takes no
  `--contracts`. So over X's lasting vacancy, which serves no bundle,
  its `redundancy-vacancy` is unobservable. That is said, and is not a
  §12 step.

**Results at 0.2 (round 58):** every check passes in all four pairings,
the owner example built from PR #858's merged head.
§3 ran 20 times in each.

`scenarios.md`'s sections:

| Pairing (A, B) | All | §1 | §2 | §3 | §4 | §5 | §6 | §7 | §8 | §9 | §10 | §11 | §12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| zk2py, zk2py | 64 | 11 | 3 | 1 | 2 | 6 | 3 | 7 | 3 | 4 | 11 | 5 | 8 |
| Rust, Rust | 62 | 11 | 3 | 1 | 2 | 5 | 3 | 7 | 3 | 4 | 11 | 4 | 8 |
| zk2py, Rust | 64 | 11 | 3 | 1 | 2 | 6 | 3 | 7 | 3 | 4 | 11 | 5 | 8 |
| Rust, zk2py | 62 | 11 | 3 | 1 | 2 | 5 | 3 | 7 | 3 | 4 | 11 | 4 | 8 |

zk2py's own sections:

| Pairing (A, B) | All | re-mint | poll | poll-miss | heal-retry | heal-retry-pause | re-mint in a campaign |
|---|---|---|---|---|---|---|---|
| zk2py, zk2py | 15 | 4 | 3 | 3 | 1 | 3 | 1 |
| Rust, Rust | 13 | 3 | 3 | 3 | – | 3 | 1 |
| zk2py, Rust | 14 | 4 | 3 | 3 | – | 3 | 1 |
| Rust, zk2py | 14 | 3 | 3 | 3 | 1 | 3 | 1 |

In all, 79, 75, 78 and 76. `poll-miss` and `heal-retry-pause` ran after
the rest, with nothing else changed but a count of complete polls.
§5's one less check, §11's and the re-mint's are zk2py's log and §11's
reason. The owner example's reason is a known deviation, F-214. The
heal-retry needs a reader cut apart from its watch, which only zk2py's
driver offers, so it runs only where B is zk2py. `heal-retry-pause`
retries in every pairing: the owner example's read ended "at its
timeout", then two complete reads, and B stepped down 5.1 s after the
heal was let go. zk2py's `--hold-heal` holds the heal's timers, so the
read it retries is the second, and it steps down at 4.0 s; the owner
example's holds every heal read, so it retries the first. `poll-miss`
passes in every pairing: B, polling through a link paused for 6 s while
A went, took over once the link was back (F-215), zk2py's B 0.35 to
0.44 s after it and the owner example's 0.12 to 0.15 s. In §3, B kept serving,
its read having missed A's claim and A then standing by on it, and in a
few runs both took over and B stepped down at the heal, 2 s on:
- zk2py, zk2py: B kept serving in 0 of 20 runs, none healed;
- Rust, Rust: 1 kept, 1 healed;
- zk2py, Rust: 5 kept, none healed;
- Rust, zk2py: 1 kept, 1 healed.

The two instance tokens reached S a median of 1 or 2 ms apart, with up
to 242 ms in a mixed pairing's first run, before calibration. The two
implementations elect the same incumbent from the same reads in every
run, start, crash, split and heal alike.

**Where zk2py and the Rust runtime differ, all reporting:**
- The owner example's `made` counts reads made, with reads not made
  apart. zk2py's driver now does the same.
- After a step-down the owner example names no `why`, which `elections.json`
  does not have. zk2py's no longer names `lower_holder`.
- The owner example's §11 refusal names a standby write, not §2.2's
  reason (F-214).
- An act that fails: the reference undoes a takeover in §2.9's order,
  makes every step of a step-down whatever one before it gave, and the
  instance then elects no more ("Decided here", 0.2). zk2py now does the
  same (`Owner.take_over`, `Owner._step_down_steps`, the candidate's
  `fault`): a marked standby holding no interface token, the first error
  kept. A test can make zk2py's acts fail at each step; nothing on the
  bus can make the owner example's fail, so that was not compared.

**What the owner example could not do from stdin** at 0.2:
- choose a value to put;
- say its zid, given since 0.3;
- say its steps, or what each poll read (its `poll_log` since 0.3);
- hold its start for a simultaneous one (F-216; `--wait-go` since 0.3,
  which never starts, F-220);
- choose a re-mint's new id;
- cut its reader apart from its watch.

Round 56's run at 0.1, zk2py against itself: 59 of 59.

In round 58, after the owner's change (its lease, what it held of an
interface forgotten at a step-down, a failed takeover undone),
`hostid_scenarios` passed 35 of 35, `state_scenarios` 15,
`freshness_scenarios` 21 and `health_scenarios` 29. In round 60 (the
owner's session opened apart from its start), `hostid_scenarios` passed
35 and `state_scenarios` 15.

**The other runners**, rerun in round 56 after the owner's change then,
against zk2py's owner:
- `hostid_scenarios` 35 of 35;
- `freshness_scenarios` 21;
- `state_scenarios` 15;
- `health_scenarios` 29;
- `config_scenarios` 51;
- `alarms_scenarios` 80;
- `telemetry_scenarios` 57;
- `budget_scenarios` 27;
- `link_scenarios` 78, in two runs;
- `desired_scenarios` 52, zk2py's author and target.

## timing.v1 (profile text 0.5, core 0.55)

```bash
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.timing_interop [--pair OWNER:READER]... [--only 1 … 6|face|gone|again|waits|missed|stray]...
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.timing_tool -c ENDPOINT [--for S] [--clock-bound MS] [--contracts P]... [--across-face [--owner-zid Z]...] zk2/<system>/<service>/**
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.timing_scenarios [--only 1 … 6|hlc|first]...
```

`timing.v1` is the tenth profile (`spec/profiles/timing/`): a stream's
period, a reader's deadline and a sample's lifespan, the three keys of
core Appendix D's interim `timing` row, which core 0.53 publishes. zk2py
read text 0.1 and core 0.53 cold in round 76 and built each part from the
text:
- **`zk2py.timing`,** session-free:
  - `declaration` and `requirement` (§2.2, §2.3): no key of the vocabulary,
    then the kind, then each key alone, period, deadline, lifespan; a
    deadline below the period declared, `below_period`;
  - `Tracker` (§2.5), one reader's deadline tracker for one provider and
    resource: every member judged at every event, in byte order, then the
    event; `missed` with its due and its detection bound max(deadline/50,
    1 ms), `met` only after a `missed`, `ended` gone or forgotten,
    `beyond_bound`; `next_due` and `next_instant`, the one-shot wait. A
    missed member stays missed when its owner is present again (F-262);
  - `LifespanReading` (§2.6), the ten steps in order, on ε alone, against
    the `meta.zid` of every descriptor held, compared by value, a clock
    falsified for the rest of the reading once a stamp of it is more than
    ε ahead;
  - `period` (§2.7), `conclude` (§5's three questions) and `rollup` over a
    resource or a run, read by each question's finding (F-264);
  - `lint`, the profile's four SHOULDs, which have no core code.

  152 of 152 fixture cases, the first time they ran.
- **The core's part.** `timing` is in the published vocabulary (W105), and
  `contracts/w105-timing-max-rate` passes. `scenarios.md`'s `pace.v1` is
  `impl/python/interop/timing/pace.v1.toml`, with the reference CLI's
  fingerprint.
- **`zk2py.timing_live`:**
  - `Reader` is a consumer of a timed resource at one address. Its trackers
    are judged at each arrival and on a one-shot wait at the earliest due.
    Presence is a `PresenceFollower`: a watch session, reads on another, a
    read 100 ms after a withdrawal. A complete read showing no member
    token of a held member ends it (F-261). It takes `meta.zid` from the
    address's instance keys' puts and from GETs. An expired sample is
    discarded, never handed on and no arrival.
  - `Window` is a tool's window over it (§2.7). It follows presence and
    holds the descriptors first, then subscribes (F-263). At the end it
    evaluates every tracker and answers §5 of every member held in the
    window (F-267). `lost` is its `StallWatch` (core §2.7).
  - `stamped_clock` is an owner's stamping clock offset from its host's
    wall clock, never its HLC (core 0.53).
- **The owner** (`zk2py.owner`) now holds core §8.1's member tokens
  (`hold_member`, `release_member`).
- **`python -m zk2py.timing_scenarios`** runs `scenarios.md` §1 to §6 on
  in-process zenoh-python routers, zk2py only: 26 of 26, in each of three
  runs. §6 runs zk2py's `Window`, the reference tool being #908's MC. No
  Rust runtime exists for `timing.v1` yet (#908's MB), so there is no
  cross pairing.
  - `hlc` measures core 0.53's facts: a stamp's time from the Unix epoch,
    an HLC moved by a later stamp it receives and re-stamping one beyond
    its delta, and a client with its HLC on stamping an unstamped put it
    receives (F-265).
  - `first` measures how often a new instance's first sample is handled
    before its descriptor (F-266).

**Text 0.2 (round 78).** zk2py read `timing.v1` 0.2 and core 0.55 cold,
and ran the scenarios against the Rust runtime:
- **`zk2py.timing`:** `Tracker` takes `epoch`: a known member of an epoch
  template waited for from its first member token seen (`member_seen`),
  never from its owner's presence, and ended by a read only once a token
  of it was seen (F-261); `beyond_bound` once while a member stays past the
  bound (F-260); `tracker_for` builds no tracker where no deadline is
  judged (F-259). 175 of 175 fixture cases (`deadlines.json` 44,
  `conclusions.json` 39).
- **`zk2py.timing_live`:** the `Reader` hands each tracker the member
  tokens its follower hears or a read shows, and records each member's
  first one seen (`member_seen_at`).
- **`python -m zk2py.timing_interop`** runs `scenarios.md` §1 to §5 in the
  four pairings of zk2py's owner and reader with the reference's black
  boxes (`owner --timing`, `consume --timing`), and §6 with zk2py's tool
  beside each owner: 84 of 84, in each of two runs.
  - zk2py's owner is driven as the black box is (`PyPace`: `cmd` paused at
    the start, an axis on once added, each member every 100 ms while on).
  - The black box reader's instants are placed on the runner's clock by
    its `subscribed` line's arrival, and its dues compared with its own
    arrivals exactly; where a due counts from its first sight of its
    owner, the runner polls its `state` every 10 ms and checks the due
    falls between the owner's start and the first `present`.
  - §4 runs the reference's owner with `--no-hlc`, its `--help`'s way of
    offsetting its stamps alone (F-273); §3 step 5 declares `x`'s member
    token from a raw session beside the reference's owner, whose `timing
    add` publishes at once (F-276).
  - Every miss was reported 0.09–0.15 ms after its due by zk2py's reader,
    0.6–1.9 ms after by the reference's, all within the 10 ms bound.

**Text 0.3 (round 79).** zk2py read `timing.v1` 0.3 cold, and ran §6 with
both tools:
- **`zk2py.timing`:** `tool_known` (a tool's known members, templates
  without parameters), `lost_for` (a loss it cannot attribute is every
  member's) and `owners_held` (across a face, `meta.zid` on the
  deployment's word alone). The tracker already read F-275 as 0.3 writes
  it. 178 of 178 fixture cases (`deadlines.json` 45, `lifespans.json` 32),
  passed unchanged.
- **`zk2py.timing_live`:** the `Window` takes no binding, so `axis/x` is
  never known to it; across a face it follows no presence, reads no
  descriptor, and holds only `owner_zids`. The `Reader` can follow no
  presence (every owner unseen).
- **`python -m zk2py.timing_tool`** is zk2py's tool as a process, beside
  `zenctl timing`: one owner's timed streams from its descriptors (the
  contracts held offline or retrieved), the window, each member's answers,
  each resource's and the run's by its finding, `timing: listening` on its
  standard error at the window's start, one JSON document, exit 0, 1 or 2.
- **`timing_interop`** drives the reference's owner by `timing declare`
  (§3 step 5) and `timing every` (§6 step 3), runs §4's owners of both
  implementations without an HLC, and checks the reference's C prints an
  expired arrival's `wall_ns`. §6 runs both tools at once beside each
  owner, each a process, and compares their answers token by token, a
  member's `known` among them; `face` runs both across a face by the
  deployment's word, with and without `--owner-zid`. The two tools agree
  everywhere but F-277: a roll-up whose every member is not asked, which
  zk2py answers unobservable (§5's sentence, `conclusions.json` #33) and
  `zenctl timing` not asked.
- **Results:** 116 checks passed in each of two runs, §1 to §5 in all four
  pairings and both tools beside both owners, and the 12 checks of F-277
  printed XFAIL, the exit 0. The checks a finding is known to fail are
  named one by one (`KNOWN`, `zk2py.checks`): any other failure fails the
  run, and a known check that passes prints XPASS.

**Text 0.4 (round 80), the final cold read.** zk2py read `timing.v1` 0.4
cold, every MUST and SHOULD against its evidence, and asked both tools
about windows the scenarios do not make:
- **`zk2py.timing`:** `rollup` reads 0.4's §5: every member set aside is
  not this profile's when every one is, else not asked, with the first
  not-asked member's reason; no member at all stays unobservable (F-277,
  resolved). 181 of 181 fixture cases: four failed until then.
- **`zk2py.timing_tool`:** a question whose run answer is set aside is set
  apart for the exit too: 0 when every question asked, at least one, is on
  its other pole.
- **`zk2py.timing_live`:** the `Window` reads "the member's end when it
  ended in the window" as its first end, whether or not it was heard again
  (F-280).
- **`timing_interop`:** round 79's twelve F-277 checks pass and left
  `KNOWN`; each tool's resource and run roll-ups are checked against
  0.4's rule at every step. Two windows beside §6, both tools beside each
  owner, are new: `gone`, a known member never heard whose owner leaves
  before its miss is due (zk2py `met` by §2.7's held time, `zenctl
  timing` `window_too_short`: F-278; a roll-up over no member, zk2py's
  reason `no_member` and none in zenctl's: F-279, both in `KNOWN`), and
  `again`, a member that ends and is heard again (F-280, where the two now
  agree).
- **Results,** in each of two runs: 138 checks passed, §1 to §5 in all
  four pairings, §6 and `face` with both tools beside both owners, and
  `gone` and `again`; the four checks of F-278 and F-279 printed XFAIL, and
  the exit was 0. No check of §1 to §6 is known to fail.
- **After review-bot on PR #939:** an end before the instant a member was
  first held in the window ends nothing (`zk2py.timing.window_held`); the
  tool sections run beside each owner the selected pairs name, with both
  tools present; `again` runs an 8 s window, the withdrawal 2 s in and
  the re-add 4 s in. §6, `gone` and `again` with both tools beside both
  owners: 48 passed, the four F-278 and F-279 checks XFAIL. F-278 to F-280
  are accepted on #908 for text 0.5, which zk2py has not read yet.

**Text 0.5 (round 81), the final cold read.** zk2py read `timing.v1` 0.5
cold, every MUST and SHOULD against its evidence and every fixture's
description against its cases, and brought its tool to it:
- **`zk2py.timing`:** the `Tracker` closes a tool's window at an `end`
  event and keeps how long it held each member (`held_ns`): its longest
  unbroken stretch, never the sum, a stretch running while the member is
  held heard, missed or waited for, so a known member never heard pauses
  at its owner's absence and starts a new stretch at its presence again
  (F-278), and its first `ended` stops it for good, so a member heard
  again is held over its first life (`first_end`, F-280). `rollup` reads
  its reason from the first member giving its answer in byte order of the
  keys, given `(key, Answer)` pairs, and none over no member (F-279);
  `rollup_answer` is the answer alone over bare tokens, where no reason is
  read, and each refuses the other's form (review-bot on PR #941). A
  window's `end` is single-use. 196 of 196 fixture cases: the fifteen new
  ones the first time they ran.
- **`zk2py.timing_live`:** the `Window` ends each tracker's window and
  reads its held time and first end from it; the period's span ends at
  the first end; only samples within the window are judged.
- **`zk2py.timing_tool`:** each roll-up carries its reason and `first`
  only when there is a member, and each member row its `span_ms`; a
  member's `ended` is its first end, and its `arrivals` those in the
  window.
- **`timing_scenarios`:** §6 steps 5 and 6, 6 of 6.
- **`timing_interop`:** §6 runs its six steps with both tools beside each
  owner, steps 5 and 6 each with a new owner; each tool's roll-ups are
  checked against 0.5's reason as well as 0.4's answer. `KNOWN` is empty:
  round 80's four F-278 and F-279 checks (`gone`) pass. Three windows
  beside §6 are new, each with both tools beside each owner: `waits`
  (three waits each shorter than the deadline, their sum longer: held the
  longest, `window_too_short`), `missed` (a missed member held while it
  stays missed, its owner gone after the miss) and `stray` (a member
  paused by its owner's absence, then heard while its owner is absent: a
  new stretch from that arrival, `met`). §4's step 2 now polls its black
  box reader until it holds the zid and was seen present, and places its
  clock with a 5 ms margin (`PLACE_MS`; 2 ms before).
- **Results,** in each of two runs: 182 checks passed, §1 to §5 in all
  four pairings (76), §6 and the six windows beside it with both tools
  beside both owners (106). No check is known to fail. The two tools
  agreed token by token at every step; their held times differ only by
  their windows' starts, 65–104 ms apart, and each one's read after a
  withdrawal. No finding is open against `timing.v1` (`SPEC-FINDINGS.md`,
  "At timing.v1 0.5 and core 0.56: the final cold read").

## arbitration.v1 (profile text 0.3, core 0.56)

```bash
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.arbitration_scenarios [--pair COMMANDERS:ACTUATORS]... [--only 1 … 11|7c|open]...
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.arbitration_tool -c ENDPOINT --contracts P... [--role R] [--for S] [SYSTEM/SERVICE]
```

`arbitration.v1` is the eleventh profile (`spec/profiles/arbitration/`): a
consumer bound to several providers of one stream follows one at a time,
the first in its binding's order whose stream arrives within its
`timing.v1` deadline and whose interface token it has not heard withdrawn,
and none, the dead-man, when none is left. Core 0.54 makes a binding an
ordered list and publishes the vocabulary, `policy` on a requirement.
zk2py read text 0.1 and core 0.54 cold in round 77 and built each part
from the text, reading every deadline through its own `zk2py.timing`:
- **`zk2py.arbitration`,** session-free:
  - `declaration` (§2.2, §2.3), the eight reasons in order, `unread` with
    no revision held, and `tool_declaration` (§2.14) over every revision a
    tool holds, `revisions_disagree` when they differ;
  - `ranking` (§2.4): by entry, then by system, then by service, byte by
    byte; the wildcard and shadowed entries;
  - `Arbiter` (§2.5 to §2.11): each sample's outcome, eligibility,
    withdrawals and re-admission (by a token or a read, never a sample),
    a move's events kept until its round trip stands and applied as one
    change, the time first at every event, the switches with their causes,
    the dead-man, the counts;
  - `conclude` (§5) and `lint` (§2.2's SHOULD, checks 1 to 7).

  115 of 115 fixture cases, the first time they ran.
- **The core's part.** `arbitration` is in the published vocabulary (W105,
  on a requirement as on a resource). `w105-arbitration-deadline`,
  `e020-requirement` and `descriptors/ok-binding-order` pass. The
  scenarios' `steer.v1` and `rudder.v1` are in
  `impl/python/interop/arbitration/`, with the reference CLI's
  fingerprints.
- **`zk2py.arbitration_live`:**
  - `Actuator` is a consumer's runtime. It refuses an invalid declaration
    before anything is declared, and a governed required interface,
    zk2py's limitation (F-270). Then it runs zk2py's owner for its
    service, and one `Arbiter` under one lock, each event at an instant of
    its monotonic clock:
    - the role's samples, on the service's session;
    - its watch, one liveliness subscriber per entry on a watch session
      of its own, without history, its round trip after declaring them;
    - each move of the watch's transport, and the round trip after it.
      A withdrawal heard while the watch session holds no link begins
      the move (F-268);
    - a read again 0.1 s after a withdrawal that leaves an address with
      no token known held, on a reader session;
    - its timer, at the active provider's due.

    It prints each switch as a JSON line, and puts each command it
    delivers on its `angle` state.
  - `Commander` puts its stream every 50 ms, holds, resumes and re-mints.
  - `ToolView` is a tool's own view (§2.14) on its own subscription,
    watch and clock.
- **`zk2py.arbitration_owner`** runs a commander or an actuator as a
  process. The actuator's standard output is its switches, as JSON lines,
  and its answers to `counts`, `reads`, `trips` and `log`.
- **`python -m zk2py.arbitration_scenarios`** runs `scenarios.md` §1 to
  §10 on an in-process router, the commanders and actuators in processes
  of their own, and §10 with zk2py's tool: 51 or 52 of 52, the one
  failure the strict first arrival of F-269. `7c` is F-268's control: the
  move begun at the transport event alone. No Rust runtime exists for
  `arbitration.v1` yet (#909's BB), so there is no cross pairing.

**Text 0.2 (round 78).** zk2py read `arbitration.v1` 0.2 and core 0.56
cold, and ran the scenarios against the Rust runtime:
- **`zk2py.arbitration`:** `conclude` answers the selection and the binding
  not asked, `no_descriptor`, with no consumer's descriptor read (F-272);
  `counts_line` lists every provider known in rank order, five counts each
  (F-271). 120 of 120 fixture cases.
- **`zk2py.arbitration_live`:**
  - the `Actuator` follows a governed required interface's incumbent
    (§2.5 outcome 6, F-270): a sample is an arrival only when stamped by
    the `meta.zid` of the lowest instance known holding the interface's
    token, its holders read once at its start after its watch's round
    trip and their descriptors got;
  - it asks the round trip after an open 100 ms after the event (core
    0.56), and prints its instants exact to the nanosecond;
  - the `Commander` leads its first put by 500 ms (F-269) and, with
    `standby`, is a `redundancy.v1` candidate whose `raw` puts go round
    its writer (scenarios §11).
- **`zk2py.arbitration_owner`** also takes the reference owner's
  `commander …` and `arbitrate counts|reads|state` words, in its JSON.
- **`steer-governed.v1.toml`,** §11's revision, written from its setup
  (the reference CLI's fingerprint, `sha256:26b19c8c…`).
- **`python -m zk2py.arbitration_scenarios --pair C:A`** runs §1 to §11
  with zk2py's (`py`) or the reference's (`rs`, the black box `owner
  --commander` / `--arbitrate`) commanders and actuators, §10 with zk2py's
  tool: every check passes in all four pairings but §8's "nothing put"
  with the reference's actuator, which puts its `angle`'s `ok` at a
  refused start (F-274), in each of two full runs. `7c` (F-268's control)
  and `open` (core 0.56's open, measured: 0 of 60 round trips asked at the
  event stood before the reopen's telling) are zk2py's alone. The
  reference's actuators' instants are placed on the runner's clock by
  their `start` line's arrival. A check an open finding is known to fail
  prints XFAIL naming it (F-274), is counted as failed and listed in the
  summary, and leaves the exit 0. With the black boxes missing, both
  runners skip the `rs` pairings with a line, or exit 2 when one was
  asked for by name; `ZK2PY_BLACKBOX` overrides their directory.

**Text 0.3 (round 79).** zk2py read `arbitration.v1` 0.3 cold, and ran §10
with both tools:
- **`zk2py.arbitration`:** `view_governed` (a view reads outcome 6 only
  when every revision held lists `redundancy.v1`) and `listened_ns`
  (listening from the last subscription to the role's stream). 120 of 120
  fixture cases, unchanged.
- **`zk2py.arbitration_live`:** `ToolView` is the `Actuator`'s runtime with
  no service: its watch, its moves kept until their round trip stands, its
  reads again and its timer, no ε, a governed interface's incumbents learnt
  as a consumer learns them, and listening counted from its subscriptions.
- **`python -m zk2py.arbitration_tool`** is zk2py's tool as a process,
  beside `zenctl arbitration`: a consumer's descriptor, each arbitrated
  role's declaring contract at the revision listed, every revision of the
  required interface held offline or retrieved from the bound providers'
  descriptors, the tool's own view, and §5's three questions, every
  selection labelled the tool's own view; with `--role` and no
  descriptor, the offline contracts that declare it (F-272). One JSON
  document, exit 0, 1 or 2.
- **`arbitration_scenarios`:** §10 runs both tools at once over each
  step's consumer, beside zk2py's and the reference's actuators, each
  checked against Expected and the reference test's exits (0, 1, 1, 1, 2,
  2), and compares them token by token. §11 runs both beside its step 2,
  holding minor 1 (governed: teleop selected, X's samples
  `not_incumbent`) and, the control, minor 0 beside minor 1 (not governed:
  safety selected). §8's "nothing put" passes with the reference's
  actuator (F-274 resolved), and §11 reads X's `standby <n>` payload
  (F-276). Every check passed in all four pairings, 329 of 329 in each of
  two runs, the two tools agreeing token by token at every step.

**Round 80, the final cold read.** zk2py read `arbitration.v1` 0.3 cold
again, every MUST and SHOULD against its evidence and every fixture's
description against its cases, and changed nothing. §1 to §9 and §11 in
all four pairings, §10 with both tools beside both actuators and §11 with
both tools beside its step 2: 329 of 329 in each of two runs, the two
tools agreeing token by token at all 24 of §10's steps a run. Offline, the
two read revisions of `steer.v1` that differ in the period alone as
`revisions_disagree`, both, and in the lifespan alone as valid, both. No
finding is open against it (`SPEC-FINDINGS.md`, "At timing.v1 0.4,
arbitration.v1 0.3 and core 0.56").

## What a complete presence read is (core 0.37, 0.38, 0.41, 0.42, 0.43, 0.44, 0.45, 0.46, 0.47, 0.49)

```bash
PYTHONPATH=impl/python ZK2PY_PROTOC=$(which protoc) \
  target/py-venv/bin/python -m zk2py.presence_scenarios [--only 1 … 10]... \
  [--owner zk2py|rust] [--reader zk2py|zenctl] [--owner-bin <owner-example>] [--zenctl <zenctl>] \
  [--crowd 2000] [--tokenless 100] [--reopen]
```

`--owner` picks who holds the tokens: zk2py's owner, or the Rust owner
example as a black box, a client of the section's router. `--reader`
picks the reader under test: zk2py's reads and follower, or `zenctl`
(`service list` for a read, `watchdog --rule 'instance-gone …'` for a
reader that follows presence live). A step a tool's command line cannot
run is reported NOT RUN, never a pass.

Core 0.37 (§8.1, §2.7, Appendix B) says what a complete presence read is.
zk2py read it cold in round 54, and implemented it from the text:
- **A read not made** (`zk2py.live`). `list_presence` makes no read
  through a session that holds no link to another node: since 0.38 it
  checks the session's links (`Session.info.links()`, which zenoh-python
  1.10.1 has), never its transports, which can still list a closing one.
  Nor does it make one on a session that has held a liveliness subscriber
  zk2py declared (`declare_liveliness_subscriber` marks it for its life).
  It reports each **not made**, with its reason (`Presence.kind`, a third
  kind beside complete and possibly incomplete). It checks the links again
  at the read's end: a read in flight when the link goes comes back
  complete and empty, and is **possibly incomplete**, cut off during it,
  its tokens seen (0.38, F-185).
- **A watch session** (`watch_session`, `settle`, `watch_or_beside`). A
  reader's liveliness subscribers go on a second session that `open_client`
  opens the same way (in no namespace since 0.42, on the wire keys), with
  one round trip there before the seed.
  `AlarmsReader`, `budget.Window`, the telemetry reader, and the alarms,
  health and hostid runners' token watches use one. A session zk2py did not
  open itself (`budget_scenarios.client`) cannot be opened again from
  zenoh-python, so its subscriber stays beside its reads, as before.
- **A withdrawal is unconfirmed until a read** (`zk2py.presence`).
  `PresenceView` is the session-free half. Its holders are an address's
  instance tokens, an interface's tokens and a member's epochs. A
  withdrawal that leaves one with nothing held makes it unobservable,
  never down, and owes a read. One read asked answers a burst. A
  make-before-break re-mint owes none. A complete read showing none makes
  it down. A read not made, or possibly incomplete, leaves it unconfirmed
  and still owed. `PresenceFollower` is its live half on a watch session.
  It reads 0.1 s after a withdrawal, and retries a read that settles
  nothing after 0.2 s, doubling up to 5 s, back to 0.2 s after a complete
  one.

**Core 0.41**, read cold in round 66, on main 2451a01d. Where zk2py
already did what the text now says, and where it did not:
- **The round trip's key.** zk2py's `settle` already asked a key no token
  holds, `zk2/@zk/settle`, but outside every subscriber's subtree, so a
  grant admitting the subscribers did not admit it. It now asks
  `settle_key(selector)`: the subscribers' selector up to `@zk`, then
  `settle` (`zk2/*/*/@zk/settle`, `zk2/lab/*/@zk/settle`).
- **A round trip that stands** (`live.RoundTrip`). `settle` counted any
  round trip that ended with its final reply and no error reply. So it
  counted one zenoh ended at the interests timeout, and one through a
  watch session cut off, which zenoh answers at once. It now stands only
  when it ended before its session's interests timeout, on a session
  holding a link before it and at its end. It makes none through a session
  cut off (F-228).
- **The interests timeout.** `list_presence` did not read a read ended
  there as incomplete: it reads like a complete one. It now reads one
  whose time reached its session's interests timeout as possibly
  incomplete (`AT_INTERESTS_TIMEOUT`), its time counted from before the GET
  was asked. zenoh-python reads no configuration back, so `open_client`
  records the value (`interests_timeout_of`); a session zk2py did not open
  is taken at zenoh's 10 s (F-232). zk2py's sessions ran with zenoh's 10 s;
  they now run with 120 s, the reference's tools' value (the SHOULD).
- **No history.** `budget.Window` and `AlarmsReader` already declared
  their subscribers without history. The follower's had history; it now
  has none, unless its watch session is in a namespace
  (`live.namespace_of`). The telemetry reader and the scenario runners'
  token watches keep theirs: each watches one address, whose history is a
  token or two.
- **A round trip stands until the watch session's transport moves.** The
  follower made one round trip at its start, and took it to stand for
  good. It now hears its watch session's transport events
  (`Session.info.declare_transport_events_listener`). A move ends the
  round trip, and leaves what the follower established unobservable
  (`MOVED`, F-229). It then asks a read, made after a new round trip. A
  read made while none stands, or across a move, is folded **unordered**:
  what it shows is up, and what it does not show is unobservable
  (`NO_ROUND_TRIP`). It is then made again. The profile readers
  (`AlarmsReader`, `budget.Window`, the telemetry reader, the desired.v1
  target, the redundancy.v1 candidate) make their round trip once, as
  before: their windows and texts are written against 0.40 or earlier.

`readings` gained 28 cases (379): a read at the interests timeout, the
round trip's key and when it stands, on fake sessions, and the view's
unordered reads and moves.

On review of PR #876 the follower changed in three ways:
- While its watch session holds no link, it makes no read beyond the one
  its move asked, and one per token move heard. The next waits for the
  transport to open, a link check every 5 s making no read. (0.42
  replaces this: no read at all, below.)
- It declares its subscriber before its transport listener, and a timer
  fired after `close` makes no read.
- An unordered read owes an ordered one, whatever it showed.

`settle` makes no GET when the watch session's links cannot be read.

**Core 0.42**, read cold in round 67, on main 0bcf1310 (#875). Where
zk2py already did what the text now says, and where it did not:
- **In a namespace, a watch session in no namespace on the wire keys**
  (F-233). zk2py did not. `watch_session` opened the watch session in
  the reading session's namespace, and the follower kept history there:
  0.41's first option, whose round trip waits for the population (131–154
  ms beside 2,000 tokens, 3.2 s beside 10,000). Now:
  - `watch_session` opens it in no namespace;
  - `watch_or_beside` and the follower declare the subscriber without
    history on the wire key (`wire_selector`: `ns/zk2/*/*/@zk/instance/*`)
    and ask the round trip on its wire key (`ns/zk2/*/*/@zk/settle`);
  - every key heard is stripped of the namespace (`strip_namespace`,
    `Stripped`); a key outside it is counted (`outside`) and never folded.

  A follower handed a watch session in the namespace keeps 0.41's
  history; its round trip's wait is the caller's (`round_trip_timeout`).
- **No round trip through a watch session cut off** (F-228). zk2py already
  did this: `settle` reads the links before and after, and asks no GET
  through a session cut off.
- **A reader whose watch session is cut off follows nothing** (F-227,
  F-228). zk2py did not. After a move it made one read on its reading
  session, folded unordered, then waited for the transport to open. Now
  it asks no GET of either session while the watch session holds no
  link. It folds the read as not made, with the watch session's reason
  (`live.WATCH_CUT_OFF`), once for each move and token move heard, and the
  transport's opening asks the next. A read whose end finds the watch
  session's links gone is folded unordered.
- **A move unconfirms what the reader holds; "a seed" is every read**
  (F-229). zk2py already did this: `PresenceView.moved` (`MOVED`), and
  every read made while no round trip stands is folded unordered.
- **A read's time, and an interests timeout zk2py cannot read** (F-232).
  zk2py already counted from before the GET is asked. It already took a
  session it did not open at 10 s, and opened its own with 120 s. It did
  not say a read's completeness rests on that. A read or a round trip on
  such a session now carries `interests_timeout_assumed`, and its
  `reading` says so (`INTERESTS_TIMEOUT_ASSUMED`).
- **The runner** follows the new steps:
  - §9 step 2 runs ten times on fresh sessions, the miss asked in one run
    at least (F-235);
  - §10's crowd is in `ns`, and the steps' own tokens are under `s10`;
  - step 5 reads `zk2/s10/*/@zk/instance/*` (F-231);
  - step 6 puts the reading session straight on R1 (F-227). The stall
    ends once the first seed has been read. Both sessions' liveliness
    GETs are counted, and the view is observed at the move itself;
  - step 7 runs one subscriber a session, a fresh token each (F-234);
  - steps 8 and 9 are new.

  zk2py's own placement of F-227, both sessions on one link, is no longer
  run.
- **The view's fold was quadratic.** zk2py found this running step 8's
  follower at a population, not from the text. `PresenceView.answered`
  grouped a read's keys holder by holder, so a read of 2,000 tokens took
  9.7 s to fold. It now groups them once: the follower's start beside
  2,000 tokens took 0.05 s, and beside 10,000 tokens 0.59 s.

`readings` gained 15 cases (394), two of them on review of PR #879.
They cover:
- the wire keys, a namespace's slashes included;
- `Stripped`;
- `watch_or_beside` in a namespace, the caller's drop hook kept;
- the follower over fake sessions: its watch session cut off, linked,
  cut while it reads, and in a namespace;
- the assumed interests timeout.

**`presence_scenarios`, every pairing**, against `owner-wd` and
`zenctl-wd` (main 0bcf1310):

| Owner, reader | §1–§6 | §7–§9 | §10 | Not run |
|---|---|---|---|---|
| zk2py, zk2py | 21 | 28 | 34 | 1 |
| Rust, zk2py | 21 | 28 | 28 | 2 |
| zk2py, zenctl | 14 | 10 | 8 | 6 |
| Rust, zenctl | 14 | 10 | 7 | 7 |

At 0.41 (round 66, against `owner-wb` and `zenctl-wb`) the same pairings
passed 80, 75, 31 and 30. Since then §9 and §10 gained checks (steps 8 and
9), and zk2py's second placement of step 6 went.

Every check passes. Not run:
- §3 step 2 (no owner breaks a member's continuity from outside);
- §10 step 7 with the Rust owner (the owner example takes no namespace);
- for zenctl: §1's act at the token, §4's handler pairings, §6 step 2's
  alive-only read (its verbs read `…/@zk/**` whole), §9, and §10 step 9
  (its watch session's GETs are not observable from its output).

§9, measured: step 2's GET at once beside a history subscriber missed
the owner's token in 10 of 10 runs in both zk2py-reader pairings. In 60
further trials each, it held the token in 0 of 60 beside zk2py's owner
and in 1 of 60 beside the Rust owner. That is the race 0.42 now states.

§10, measured on zenoh-python 1.10.1 routers (a release build), the crowd
in `ns`:
- **Steps 1 to 3** as at 0.41. The round trip was still waiting at the
  stall's end (190–200 ms), and the withdrawal was heard 10 of 10. Without
  the round trip it was never heard, 3 of 3. Through R2, the same.
- **Step 4, in `ns`.** Without history the round trip took 1.6–2.2 ms
  beside 2,000 tokens and 14.8 ms beside 10,000. With history it took
  128–224 ms beside 2,000, and 3.4–3.8 s beside 10,000. Every token was
  delivered by its end with `indirect=False`. With the default callbacks,
  the end came first in 0–1 of 10 runs (2,000) and 1 of 3 (10,000).
- **Step 5.** The read of `s10`, with no token held, was complete in
  1.6 ms beside either crowd. Stalled, zenoh ended the read at 302 ms and
  the round trip at 301 ms. zk2py read the read possibly incomplete, and
  its round trip not made.
- **Step 6, the reading session direct.**
  - The stall lasted 304–308 ms until the first seed. Its one round trip
    ran out at 302 ms. The seed was unordered: the owner up, an absent
    address unobservable.
  - The retry's round trip stood (1 ms), and the read was complete.
  - At the cut, the view held the owner and the absent address
    unobservable (`MOVED`) at the move itself, before any read. The reread
    was not made, with the watch session's reason. Over 3 s cut, 0 GETs
    went through either session.
  - After the heal, a round trip stood before the read that found the
    owner up. In all: 3 round trips asked and 4 reads, 1 of them not made.
- **Step 7.** A watch session in `ns` without history never heard the
  token go. With history it did, and so did one in no namespace on the
  wire key. zk2py's follower in `ns` watches from no namespace, on the
  wire key, without history, and reads the owner down. Handed a session
  in `ns`, it keeps history and reads it down. Its control without
  history goes on reading it up.
- **Step 8.** From no namespace on `ns/zk2/*/*/@zk/instance/*`, the
  round trip on `ns/zk2/*/*/@zk/settle` took 0.5–0.8 ms beside 2,000
  tokens and 2.7 ms beside 10,000, carrying none. The crowd token's
  withdrawal was heard on its wire key. zk2py's follower so wired:
  - its round trip stood in 0.5–0.6 ms (2,000) and 2.9 ms (10,000);
  - its seed was complete;
  - it read a withdrawn crowd address down after its re-read.
- **Step 9.**
  - Through the watch session cut off, `settle` asked no GET, and none
    stood.
  - Cut 150 ms into the wait, zenoh's own round trip came back at 151–154
    ms, with the final reply alone and no error reply.
  - zk2py's came back at 151 ms and did not stand (`CUT_OFF_DURING`).
- **10,000 tokens** (the largest asked) fit on this VM: §10 ran whole
  there, 34 of 34 with zk2py's reader and 8 of 8 with zenctl.
- **zenctl.**
  - It read the 2,000 in `ns` complete in 1.1 s. It read the 10,000
    complete with `--timeout 30` (26.6 s in all); with `--timeout 2`,
    2,638 rows, "possibly incomplete".
  - Step 5's read of `s10` was complete under a 300 ms interests timeout
    on a short path, `"complete": false` behind a 0.5 s path, and
    complete on its own interests timeout.
  - `watchdog`, behind a 0.4 s path at its start, then cut and healed:
    - it was never firing;
    - while cut it was unobservable ("no presence read is made");
    - it was ok after the heal.
  - In `ns` (step 7) it read the owner gone.
  - Beside the crowd (step 8), it read `crowd/c5` ok after 0.6 s, and
    firing 0.5 s after the token's withdrawal, at 2,000 and at 10,000.

**Where the two implementations disagree:** on no verdict.
- **Wording.** zenctl says a read ended at the interests timeout "ran to
  its timeout … raise --timeout to ask again". zk2py names the interests
  timeout.
- **Round trips in step 6.** zk2py asks one round trip per attempt, so
  one runs out before its first seed. The reference asks two, the
  start's and the seed's. 0.42's step 6 states the reference's count
  (F-236).

`live_interop` against `owner-wd`: 367 of 367. The consume example is
not built here, so the runs that need it were left out with the new
`--skip owner --skip freshness --skip health`. The other runners pass
again against `owner-wd` and `zenctl-wd`:
- telemetry 163 (`--tool`);
- hostid 35;
- health 29;
- freshness 21;
- alarms `--quick` 78 (one not run);
- state 15;
- budget 41;
- config 51;
- link 89 (in two runs, §1–§10 and §11–§13);
- desired 85 (both parties the owner example, in two runs);
- redundancy §1–§12 89 (A and B the owner example).

**Core 0.43**, read cold in round 68, on main 7efffad0 (#880). It
changes the wording of two of §10's steps, and no rule:
- **Step 6's stall ends at the reader's first seed** (F-236). By then
  every round trip the reader asked before that seed has run out, and
  how many is the reader's own. zk2py's runner already ended the stall
  there. It now checks every round trip asked before the seed, not only
  the first: zk2py asks one, and it ran out at 302 ms.
- **Step 9's round trip waits 2 s, and the link is cut 150 ms into it**
  (F-237). zk2py's runner already did this. Both round trips came back at
  150–151 ms: zenoh's with the final reply alone, and zk2py's not
  standing.

zk2py's code did not change: what 0.43 says of steps 6 and 9 is what its
runner did at 0.42.

Reading §8.1 and `scenarios/presence.md` end to end turned up three
findings:
- **F-238.** Step 6's stall ends at an event that some readers never
  produce. A reader that makes no read while no round trip stands never
  seeds while the link is stalled.
- **F-239.** §8.1 says a subscriber without history is "never told" of
  a token declared while its transport was down, and that its round trip
  on the wire keys is "one round trip at any population". Measured: when
  the transport opens again, the subscriber is told every token it
  matches, and the next round trip waits for them all, as a history
  subscriber's does.
- **F-240.** A round trip that ran out at the interests timeout is read
  "as not made", although its GET was asked.

With `--reopen`, `presence_scenarios` §10 ends with two notes for F-239.
They measure the reopen beside the crowd. They are not checks of a step,
and a measurement that fails becomes a note. Without the flag they do not
run.

**The step 7 control at 10,000.** The control is a follower in `ns`
without history, which should never hear the owner go. Beside 10,000
tokens it read the owner down in the first 2 of 8 runs. Those runs did
not record whether a read or a withdrawal heard had done it. The runner
now waits until the follower owes no read before the owner withdraws,
and its detail lists the follower's reads, the withdrawals it heard and
its round trips. It passed in the 6 runs since, each with one read before
the withdrawal and no withdrawal heard. This is zk2py's own control, not
a step's Expected, and the cause of the two failures is not known.

**`presence_scenarios`**, against `owner-wd` and `zenctl-wd` (main
7efffad0). §1–§8 ran once, zk2py's owner and reader, and §9–§10 ran in
every pairing:

| Owner, reader | §1–§8 | §9 | §10 | Not run |
|---|---|---|---|---|
| zk2py, zk2py | 40 | 9 | 34 | 1 |
| Rust, zk2py | – | 9 | 28 | 1 |
| zk2py, zenctl | – | – | 8 | 2 |
| Rust, zenctl | – | – | 7 | 3 |

Every check passes. Not run:
- §3 step 2, as before;
- §10 step 7 with the Rust owner;
- for zenctl, §9 and §10 step 9.

§10 also ran whole beside 10,000 tokens with zk2py's reader. The last
six runs passed 34 of 34; the two before them failed the step 7 control,
as above.

§10 at 0.43, measured on zenoh-python 1.10.1 routers, the crowd in `ns`:
- **Step 6.** The stall lasted 304 ms until the first seed. zk2py's one
  round trip before it ran out at 302 ms, and the seed was unordered. On
  the retry, the round trip stood in 1 ms and the read was complete. In
  all: 3 round trips and 4 reads, 1 of them not made at the cut.
- **Step 9.** Through the watch session cut off, no GET was asked. Cut
  150 ms into a 2 s wait, zenoh's round trip came back at 151 ms, and
  zk2py's at 150–151 ms, not standing.

**F-239, measured.** The watch session is in no namespace and reaches
R1 through a relay link. Its subscriber has no history and is on
`ns/zk2/*/*/@zk/instance/*`:

| | 2,000 tokens | 10,000 tokens |
|---|---|---|
| Round trip at its start, tokens told | 0.6–47 ms, 0 | 41–50 ms, 0 |
| Tokens told again after a cut and a heal | all | all |
| Round trip after the reopen | 134–145 ms | 3.2–4.7 s |
| Control: a history subscriber's round trip at its start | 125–137 ms | 2.8–3.8 s |

- **In no namespace,** with three tokens, the same held in 3 of 3 runs.
  After the heal the subscriber was told three tokens:
  - one declared before the subscriber, which it never heard withdrawn
    at the cut;
  - one held through the cut;
  - one declared while the link was cut.

  A token withdrawn while the link was cut was not told again.
- **zk2py's follower.** With only its watch session through the link, it
  read complete again 0.15 s after the reopen beside 2,000 tokens, and
  3.3–3.7 s after it beside 10,000. With both sessions through the link
  and a 5 s round-trip wait, beside 10,000 tokens, its round trip after
  the reopen took 4.2–4.5 s. Its read was complete 4.3–4.6 s after the
  heal.
- **`zenctl watchdog --namespace ns --rule 'instance-gone crowd/*'
  --timeout 30`**, with every session through the link, beside 10,000
  tokens:
  - it was ok 19.3–19.7 s after it started;
  - after a 3 s cut it was unobservable;
  - after the heal it was still unobservable at 120 s and at 420 s, in
    2 of 2 runs.

  Its stderr carried zenoh's `Unknown interest` errors and `Didn't
  receive DeclareFinal for interest … Timeout(120s)!`. Beside 2,000
  tokens it was ok 1.5 s after the heal. Watching one address
  (`crowd/c7`) beside 10,000 tokens, it was ok 0.47 s after the heal.

**Where the two implementations disagree:**
- **Wording, as at 0.42.** zenctl says a read ended at the interests
  timeout "ran to its timeout … raise --timeout to ask again". zk2py
  names the interests timeout.
- **Step 6's round trips.** These differ as 0.43 now allows: one before
  the first seed for zk2py, and two for the reference.
- **Beyond the scenarios** (F-239): beside 10,000 tokens,
  zk2py's follower read complete again within 5 s of a heal. zenctl's
  watchdog over every address did not within 420 s.

**Core 0.44**, read cold in round 69, on main c178e18f (#882). It
corrects what a move leaves (F-239), adds two SHOULDs, rewords step 6's
stall (F-238) and a round trip at the interests timeout (F-240), and adds
§10 step 10. Where zk2py already did what the text now says, and where it
did not:
- **A token declared while the transport was down is told at the reopen**
  (F-239). zk2py already read a put heard after a move as any put heard:
  evidence the token is held, never evidence of what went. Nothing
  changed.
- **"A reader SHOULD wait for the round trip after a move."** zk2py did
  not. Its follower waited `round_trip_timeout` (1 s by default) for every
  round trip, so beside 10,000 tokens the one after a reopen ran out and
  was asked again. Now, after the first move it waits until the round trip
  ends (`move_wait_s`, None by default): at the latest at the watch
  session's interests timeout, 120 s for zk2py's sessions.
- **"One round trip, and one read, at a time."** zk2py did not.
  - Each GET was a blocking call with its own timeout. A read whose
    timeout ended was retried 0.2 s later, beside the one its router still
    answered in full.
  - Measured beside 10,000 tokens: a read whose own 0.2 s timeout ended
    made the next GET on its session take 258–266 ms, where a GET on a
    quiet session takes 6–10 ms.
  - Now `live.Flight` asks each GET on a thread of its own, its own
    timeout above the session's interests timeout
    (`live.flight_timeout`), so the caller's wait can end while the GET
    runs on.
  - `live.Barrier` gives a watch session's round trips through flights, one
    per transport state: in flight, or ended and not yet handed back, it
    is taken up; after a move the next caller asks anew; one asked before
    a move stands for nothing after it (`MOVED_DURING`).
  - A round trip that stood stands until the transport moves, however long
    ago it ended. But a move count holds only the moves its listener has
    told. So one taken up after it ended stands only if the watch session
    still holds a link (`CUT_OFF_SINCE` otherwise; review-bot on PR #885).
  - The move count is one counter. The follower's barrier reads the
    follower's count (`Barrier(gen=…)`), and refuses `moved()`. The runner's
    barrier, which has no follower, counts the moves it is told.
  - The follower's round trips go through that barrier, and its reads
    through a flight of its own. A wait that ends first (`WAIT_ENDED`, or
    `read()` giving None) leaves the GET running, and the next attempt,
    0.2 s later, takes it up. A read that raised raises in every caller
    that takes it up.
  - `PresenceFollower.next_move(wait, since=…)` lets a caller wait for the
    view's next change as long as it likes, and never miss one between two
    calls. The reads run on the follower's own timers, so a wait that
    ends drops nothing.
- **A round trip at the interests timeout reads "as not standing"**
  (F-240). `RoundTrip.made` is now True for one, since it was asked.
  `stands` was already False.
- **Step 6's stall** (F-238). zk2py's follower seeds after a round trip
  that ran out, so the stall still ends at its first seed. The 300 ms is
  the watch session's interests timeout, as zk2py's runner already set it.

The profile readers (`AlarmsReader`, `budget.Window`, the telemetry
reader, the desired.v1 target, the redundancy.v1 candidate) keep their
one-shot round trip and blocking reads. Their texts are written against
0.40, as 0.44 leaves them.

`readings` gained 15 cases (409), five of them on review of PR #885. They
run on fake sessions whose GETs end when the case opens their gate, so a
slow runner can make a case wait, never fail it. They cover:
- a round trip asked four times, three given up on at once: one GET;
- a move while one is in flight: a new one, and `MOVED_DURING`;
- a round trip taken up after it ended: standing with a link, and
  `CUT_OFF_SINCE` without one;
- one move counter;
- a read whose wait ended, taken up by the next with no second GET;
- a read's error handed on to a second caller;
- the round trip after a move waited for;
- `next_move`.

**The runner** gains step 10, and a `Watchdog` run for zenctl:
- **Step 10's watch session** is in no namespace, through a link, its
  interests timeout 60 s, its subscriber without history on
  `ns/zk2/*/*/@zk/instance/*`, with direct callbacks (F-230). The reader
  under test's round trip is a `live.Barrier` on it, told each transport
  move, and both sessions' GETs are counted.
- **The reader under test** is zk2py's follower in `ns`. Its watch session
  is in no namespace, and both of its sessions go through one link. A
  caller thread calls `next_move(0.02)` in a loop. The checks read every
  GET's span on both sessions: when it was asked, and when its final reply
  came.
- **zk2py's own analog of a dropped call** is the same follower with every
  wait of its own at 20 ms: `timeout`, `round_trip_timeout` and
  `move_wait_s`. This makes its take-up path run. It is a check, labelled
  as zk2py's own.
- **zenctl.** `watchdog --namespace ns --rule 'instance-gone crowd/*'
  --timeout 30`, every session through one link, cut for 1 s. Its one
  round trip, and no read beside one in flight, are NOT RUN: they are not
  observable from its output. Its stderr's zenoh lines are said as a
  proxy. The four round trips are NOT RUN for zenctl too.
- **`--reopen` with zenctl** reruns round 68's watchdog run: a 3 s cut, a
  note.
- **Two fixes of zk2py's own, not the text's:**
  - `_Counted` now times every GET's span;
  - `live_interop`'s budget run now waits up to 5 s for the owner
    example's 10 health checks before its first GET. health.v1 puts only
    the status before the tokens, and one run in four read 0 members at
    once after `ready`.

**`presence_scenarios`**, against `owner-wf` and `zenctl-wf` (main
c178e18f). §1–§8 ran once, with zk2py's owner and reader. §9 and §10 ran
in every pairing:

| Owner, reader | §1–§8 | §9 | §10 | Not run |
|---|---|---|---|---|
| zk2py, zk2py | 40 | 9 | 46 | 1 |
| Rust, zk2py | – | 9 | 40 | 1 |
| zk2py, zenctl | – | – | 9 | 4 |
| Rust, zenctl | – | – | 8 | 5 |

Every check passes. Not run:
- §3 step 2, as before;
- §10 step 7 with the Rust owner;
- for zenctl, §9, §10 step 9, and step 10's four round trips, its one
  round trip after the reopen and its reads.

§10 also ran whole beside 10,000 tokens: 46 of 46 with zk2py's reader,
and 9 of 9 with zenctl.

§10 at 0.44, measured on zenoh-python 1.10.1 routers, the crowd in `ns`:
- **Step 6.** As at 0.43: the stall lasted 303–306 ms until the first
  seed, and the one round trip before it ran out at 301 ms. In all, 3
  round trips and 4 reads, 1 of them not made at the cut.
- **Step 10, the watch session.**

  | | 2,000 tokens | 10,000 tokens |
  |---|---|---|
  | Round trip at its start | 42–46 ms, no token | 41–52 ms, no token |
  | Told at the cut | no withdrawal | no withdrawal |
  | Told at the reopen | every token | every token |
  | Round trip after the reopen | 147–160 ms | 3.7–4.0 s |
  | GETs for the four asks | 1 | 1 |

  - "Every token" includes the one declared while the link was cut. The
    one withdrawn then was never told.
  - The four asks: the first three came back at once, `WAIT_ENDED`, and
    the fourth took the round trip up and stood. The last token was told
    0–4.1 ms before the round trip's end.
  - With zenoh-python's default callbacks too, every token had been
    handed over by the round trip's end: 5 of 5 runs at 2,000, 3 of 3 at
    10,000.
  - Only the first round trip after a reopen waits. The next, asked at
    once after it, took 0.3–0.8 ms beside 2,000 tokens and 0.8–1.7 ms
    beside 10,000 (F-241).
- **Step 10, zk2py's follower.** One round trip after the reopen, none
  while cut off, and no GET beside one in flight, in every run:

  | | 2,000 tokens | 10,000 tokens |
  |---|---|---|
  | Complete again after the heal | 0.19–0.21 s | 4.2–4.9 s |
  | Its round trip after the reopen | 49–57 ms | 3.5–4.3 s |
  | Reads in all, its seed included | 2 | 2 |

  - At the cut, every crowd address was unobservable (`MOVED`) at the
    move itself.
  - Its caller's 20 ms loop made 190–8,600 calls from the cut. Most were
    woken by the tokens told at the reopen.
- **zk2py's own analog, every wait 20 ms.** One round trip after the
  reopen, and no GET beside one in flight:

  | | 2,000 tokens | 10,000 tokens |
  |---|---|---|
  | Complete again after the heal | 0.79–2.79 s | 7.6–7.7 s |
  | Reads in all, its seed included | 3 | 6 |
  | Reads taken up | 3 | 32–34 |
  | Round trips taken up | 1 | 17–18 |

  The reads before the last were unordered: they were asked while the
  round trip was in flight, its wait having ended.
- **zenctl, step 10's reader.** It was unobservable from the cut, never
  firing, and ok again 2.5 s after the heal beside 2,000 tokens. Beside
  10,000 it was ok 25.2 s and 27.7 s after the heal. Its stderr carried no
  `Unknown interest` and no `Didn't receive DeclareFinal`.

**Round 68's run, again.** `zenctl-wf watchdog --namespace ns --rule
'instance-gone crowd/*' --timeout 30`, every session through the link,
beside 10,000 tokens, a 3 s cut and a heal, in 3 of 3 runs:
- it was ok 21.0–23.9 s after its start;
- it was unobservable 0.4–0.5 s after the cut;
- it was ok again 25.2–27.1 s after the heal;
- its stderr carried no `Unknown interest` and no `Didn't receive
  DeclareFinal`.

With `zenctl-wd` (round 68) it was still unobservable at 420 s, its stderr
full of both. The time is about what the reference measured for a debug
build (28–29 s for the fleet's watchdog, 32 s for zenctl).

**Where the two implementations disagree:** on no verdict.
- **Wording.** zenctl now says a read "ran to its timeout or to its
  session's interests timeout (routing/interests/timeout)", raise
  "--timeout, and the interests timeout above it". zk2py names the
  interests timeout. Round 68's difference is gone.
- **Step 6's round trips** still differ, as 0.43 allows: one before the
  first seed for zk2py, and two for the reference.
- **The time to recover beside 10,000 tokens.** After the heal, zk2py's
  follower was complete again in 4.2–4.9 s, and the debug zenctl in 25–28
  s. Both are within step 10's 30 s, which the step asks beside 2,000.

`live_interop` against `owner-wf` and `consume-wf`, the consume example
built this round: 426 of 426. Its first run passed 425: the one failure
was zk2py's own runner, fixed as above.

**Core 0.45**, read cold in round 70, on main 82858b16 (#886). It is
wording only: it answers round 69's four findings, adds one MAY and one
Appendix B fact, and changes no fixture and no verdict. zk2py's readers
already did what it says, and none of them changed:
- **Only the first round trip after each reopen waits** (F-241). The
  follower waits for the round trip after a move (`move_wait_s`), and asks
  none after one that stood until the next move.
- **A wait that ends first takes up the round trip in flight** (F-242),
  asking another only once that one has ended without standing. This is
  `live.Barrier`. `readings` gains two cases for it (411).
- **In flight is at the reader, and the MAY** (F-243). `live.Flight` asks
  each GET with its own timeout above its session's interests timeout
  (`live.flight_timeout`), and the caller's wait is apart. zk2py does this
  for its follower's GETs and the runner's barrier. Its profile readers
  keep their blocking GETs, whose own timeout is their wait (F-247).
- **§10 step 10's counts are the reader's own** (F-244). zk2py's runner
  already counted them at the reader's own sessions.

**The runner** follows 0.45's step 10:
- **The reader under test** is zk2py's follower with its own waits each
  ended after 20 ms (`timeout`, `round_trip_timeout`, `move_wait_s`). Its
  reads run on its own timers, not on its caller's call, so 0.45 has these
  waits stand in for the dropped calls. At 0.44 this was "zk2py's own
  analog". The follower with its waits as zk2py's callers set them still
  runs beside it, now labelled zk2py's own.
- **The four round trips** name the reader's own interface, `live.Barrier`.
  A note after them gives the next round trip, zenoh's own, asked at once
  (F-241's "once that has been sent the next is one round trip again").
- **zenctl.** The step's check is now 0.45's one judgement of a tool run as
  a black box: its read complete again within 30 s of the heal. What its
  output shows from the cut, unobservable and never firing, is a check of
  its own, which 0.45's "alone" leaves out (F-245). The four round trips
  and zenctl's counts are notes, no longer NOT RUN: 0.45 does not ask them
  of a black box.
- **`--reopen`** adds a measure in which the first round trip after the
  reopen is asked only once the telling is over (F-241).
- **A fix of zk2py's own, step 7's control.** The control is a follower in
  `ns` without history, which should never hear the owner go.
  - In this round's first §10 run it heard the withdrawal, and failed (45
    of 46). Round 68 saw the same twice beside 10,000 tokens, cause
    unknown.
  - A probe of 40 trials of the control as it was found the cause. The
    withdrawal was heard in 3, and in each the subscriber had first been
    told the token: the holder's declaration had reached the router after
    the subscriber. zenoh orders one session's messages, never two
    sessions'.
  - Step 7 runs "as step 1", where the tool reads the token before the
    watch session opens. The control did not. It now waits until the
    tool reads the token, and its detail says whether the subscriber was
    told it. In the 5 runs since, it never was, and the control passed.

**`presence_scenarios`**, against `owner-wf` and `zenctl-wf` (main
82858b16). §1–§8 ran once, with zk2py's owner and reader. §9 and §10 ran
in every pairing:

| Owner, reader | §1–§8 | §9 | §10 | Not run |
|---|---|---|---|---|
| zk2py, zk2py | 40 | 9 | 46 | 1 |
| Rust, zk2py | – | 9 | 40 | 1 |
| zk2py, zenctl | – | – | 10 | 2 |
| Rust, zenctl | – | – | 9 | 3 |

Every check passes, once step 7's control waits as above. zk2py's §10 ran
four times beside 2,000 tokens, 46 of 46 in the last three. Not run:
- §3 step 2, as before;
- §10 step 7 with the Rust owner;
- for zenctl, §9 and §10 step 9.

zenctl's §10 has one check more than at 0.44, its step 10 check now being
two, and two fewer NOT RUN.

**Beside 10,000 tokens**, §10 ran whole:
- 46 of 46 with zk2py's reader, twice (once with `--reopen`);
- 10 of 10 with zenctl, step 9 not run.

§10 step 10 at 0.45, measured on zenoh-python 1.10.1 routers (a release
build), the crowd in `ns`:

| | 2,000 tokens (5 runs) | 10,000 tokens (2 runs) |
|---|---|---|
| The round trip at the start | 40.9–41.7 ms, no token | 41.5–41.7 ms, no token |
| The first after the reopen | 148–154 ms | 3.80–3.81 s |
| The next, asked at once (F-241) | 0.56–0.88 ms | 1.67–1.69 ms |
| GETs for the four asks | 1 | 1 |
| Stand-in follower: complete after the heal | 0.79–0.80 s | 7.09–7.10 s |
| Stand-in follower: reads, reads taken up | 3, 3 | 6, 29–30 |
| Default follower: complete after the heal | 0.18–0.21 s | 3.97–5.11 s |
| zenctl watchdog: ok again after the heal | 1.41–1.49 s | 25.3 s |

- Beside 10,000 the last token was told 21 ms before the round trip after
  the reopen ended.
- Each follower asked one round trip after the reopen and none while cut
  off, and no GET beside one in flight.
- zenctl's stderr held no `Unknown interest` and no `Didn't receive
  DeclareFinal` line in any run.

**Measured for the cold read** (scratch probes on the same routers, and
`--reopen`). These are notes, not checks:
- **Which round trip after a reopen waits** (F-241). The watch session is
  in no namespace, through a link, its subscriber without history on the
  wire key:

  | First after the reopen, asked | 2,000 tokens | 10,000 tokens |
  |---|---|---|
  | at once, once the open is heard | 136–170 ms | 3.7–4.8 s |
  | from the transport event itself | 155–165 ms | 4.7–4.9 s |
  | once the telling is over | 0.6–1.0 ms | 1.8–2.4 ms |

  The next was 0.23–2.9 ms in each case. Even when asked from the
  transport event, the first waited for the telling, in 5 of 5 runs. So a
  round trip asked after the reopen ends after the telling, and one asked
  once the telling is over is one round trip.
- **A GET its own timeout ends** (F-243), beside 10,000 tokens. A read with
  a 0.2 s timeout ended with one error reply, `Timeout`, and its final
  reply. Nothing came after them within 6 s, in 3 of 3 runs. The GET asked
  next on that session waited for what was left of the router's answer:

  | | after a later read | after the session's first read |
  |---|---|---|
  | a round trip | 266–280 ms | 2.6–3.4 s |
  | a full read | 783–792 ms | 3.0–3.4 s |

  A round trip on a quiet session took 1.6–2.7 ms (F-246).
- **A session's first read of the population** (F-246). Beside 10,000
  tokens the first full read took 3.3–4.2 s, on 11 fresh sessions. A
  later read took 0.52–0.58 s. A read after the session's transport
  reopened was a later read: 0.55 s, in 2 of 2.
- **A GET whose own timeout equals the interests timeout** (the MAY's
  "at or above"). Both were 1 s, through a stalled link. zenoh's runtime
  ended it at 1.001–1.002 s with no error reply, in 5 of 5.

**Where the two implementations disagree:** on no verdict.
- **Step 6's round trips** still differ, as 0.43 allows: one before the
  first seed for zk2py, and two for the reference.
- **The time to recover beside 10,000 tokens.** zk2py's follower was
  complete again 4.0–5.1 s after the heal. With its waits at 20 ms it took
  7.1 s. The debug zenctl took 25.3 s. All are within step 10's 30 s.

**Core 0.46**, read cold in round 71, on main 0893c8fc (#888). It is
wording only: it answers round 70's three findings, and changes no
fixture, no verdict and no behaviour. zk2py's readers did not change, and
`readings` is unchanged (411):
- **A black box is judged on what its output shows** (F-245): every
  address it followed unobservable from the cut, never down, and its read
  complete within 30 s of the heal.
- **The wait after a move is a session's first read** (F-246). The
  follower sizes nothing on a read's cost: after a move its round trip
  waits until it ends, at the latest at its watch session's interests
  timeout.
- **The MAY's example is zk2py's follower** (F-247), as its README said.

**The runner** follows 0.46's Expected 10:
- **zenctl's step 10 is one check**, where at 0.45 it was two. It is
  unobservable from the cut, never firing, and ok again within 30 s of the
  heal. zenctl's watchdog prints a transition only when it evaluates, once
  every `--every`. The runner sets 0.5 s, below the 1 s cut, and reads
  "from the cut" as its first transition after the cut, printed before the
  heal (F-249).
- **What it reads** (review-bot on PR #889). zenctl prints one row per
  rule, never one per address: `instance-gone crowd/*` is one state for
  every address it follows, firing while any is gone. The check reads
  every transition after the cut up to the first `ok` printed after the
  heal. The first must go from `ok` to `unobservable` before the heal, and
  every one before that closing `ok` must be `unobservable`: none firing,
  and no `ok` before a complete read after the heal. The cut waits until
  the watchdog has printed nothing for two evaluations, so no row printed
  before the cut is read as the cut's.
- **`--reopen`** with zenctl adds F-249's note: the same run at the
  watchdog's default `--every 5`, and at 2 s.

**`presence_scenarios`**, against `owner-wf` and `zenctl-wf` (main
0893c8fc). §1–§8 ran once, with zk2py's owner and reader. §9 and §10 ran
in every pairing:

| Owner, reader | §1–§8 | §9 | §10 | Not run |
|---|---|---|---|---|
| zk2py, zk2py | 40 | 9 | 46 | 1 |
| Rust, zk2py | – | 9 | 40 | 1 |
| zk2py, zenctl | – | – | 9 | 2 |
| Rust, zenctl | – | – | 8 | 3 |

Every check passes. zenctl's §10 has one check fewer than at 0.45, since
its two step 10 checks are now one. Not run:
- §3 step 2, as before;
- §10 step 7 with the Rust owner;
- for zenctl, §9 and §10 step 9.

**Beside 10,000 tokens**, §10 ran whole: 46 of 46 with zk2py's reader,
twice (once with `--reopen`), and with zenctl 9 of 9 twice and 8 of 9 once
(with `--reopen`), step 9 not run. In that run zenctl was ok again 30.9 s
after the heal, past step 10's 30 s. The step asks the 30 s beside its
2,000 tokens, not beside 10,000, where the debug zenctl sits at the edge:
24.6–30.9 s here, 25.3 s in round 70 and 25–28 s in round 69.

§10 ran once more in each pairing after review-bot's items on PR #889,
the default crowd: 46 of 46 with zk2py's reader, 9 of 9 with zenctl and
zk2py's owner, and 8 of 8 with zenctl and the Rust owner.

§10 step 10 at 0.46, measured on zenoh-python 1.10.1 routers (a release
build), the crowd in `ns`:

| | 2,000 tokens (3 runs; zenctl's 5) | 10,000 tokens (2 runs; zenctl's 3) |
|---|---|---|
| The round trip at the start | 41.6–43.8 ms, no token | 41.0–44.1 ms, no token |
| The first after the reopen | 154–241 ms | 3.76–3.81 s |
| The next, asked at once (F-241) | 0.61–1.09 ms | 1.72–1.99 ms |
| GETs for the four asks | 1 | 1 |
| Stand-in follower: complete after the heal | 0.80–2.79 s | 7.31–8.08 s |
| Default follower: complete after the heal | 0.19–0.26 s | 4.25–6.90 s |
| zenctl watchdog: unobservable after the cut | 0.46–0.50 s | 0.45–0.48 s |
| zenctl watchdog: ok again after the heal | 1.46–1.50 s | 24.6–30.9 s |

- zenctl's first transition after the cut was unobservable in every run,
  its evidence naming every address under `crowd/*`. It never fired.
- Beside 10,000 the stand-in follower asked 6 and 3 reads, and took up
  31 and 22. Its round trip after the reopen took 5.4 s and 3.85 s, the
  default follower's 3.6 s and 4.06 s.
- zenctl's stderr held no `Unknown interest` and no `Didn't receive
  DeclareFinal` line in any run.

**Measured for the cold read** (scratch probes on the same routers, and
`--reopen`). These are notes, not checks:
- **A session's first read, and a reader's seed** (F-248). Beside 10,000
  tokens a fresh session's first read took 3.48–3.77 s and its next
  0.49–0.50 s, in 3 of 3. zk2py's follower on fresh sessions seeded in
  3.13–3.91 s, a session's first read, and the next read on its reading
  session took 0.47–0.49 s, in 3 of 3.
- **Successive reopens** (F-246). A watch session in no namespace, its
  subscriber without history on the wire key, its link cut and healed
  three times beside 10,000 tokens. The round trip after each reopen took
  3.43–4.54 s, and the next 1.6–1.9 ms, on 2 sessions. Every reopen costs
  about a session's first read, not only the first. Beside 2,000, at the
  second cut the subscriber was told all 2,000 tokens withdrawn before the
  heal, since it had been told them at the first reopen. After the heal it
  was told all 2,000 again.
- **The watchdog's period** (F-249). The step's run with zenctl's
  watchdog at its default `--every 5` printed nothing after the 1 s cut,
  in 3 of 3 runs beside 2,000. At `--every 2` it printed unobservable 1.91
  s after the cut, after the heal, and ok 3.92 s after it, in 3 of 3.
  Beside 10,000, where it took about 30 s to recover, it printed
  unobservable 4.97 s after the cut at 5 s, and 1.98 s after it at 2 s.

**Where the two implementations disagree:** on no verdict.
- **Step 6's round trips** still differ, as 0.43 allows: one before the
  first seed for zk2py, and two for the reference.
- **The time to recover beside 10,000 tokens.** zk2py's follower was
  complete again 4.25–6.90 s after the heal, and 7.31–8.08 s with its
  waits at 20 ms. The debug zenctl took 24.6–30.9 s, once past the 30 s
  that step 10 asks beside 2,000.

**Core 0.47**, read cold in round 72, on main 8880e3c6 (#891). It is
wording only: it answers round 71's two findings, and changes no fixture,
no verdict and no behaviour. zk2py's readers did not change, and
`readings` is unchanged (411):
- **A reader's seed is the first read it can time** (F-248). The follower
  sizes nothing on a read's cost. Step 10's notes now give each
  follower's seed beside its round trip after the reopen, in the same run.
- **A black box is run with its output's period at most half the cut**
  (F-249). The runner's `--every 0.5` beside the 1 s cut is half the cut.
  Step 10's check now refuses a period above half the cut. It judges
  zenctl from its first transition after the cut, which must come while
  the link is cut, through the first `ok` after the heal.

**The runner had a fault of its own.** Step 10's "the round trip after
the reopen ends only after them" took the round trip's end as its ask
plus its elapsed time. The ask is stamped before the links check. The
elapsed time runs from just before the GET to the flight thread's wake-up.
So the sum is off the final reply by a few ms, either way. In this round's
first run the check failed by 4.4 ms. The end is now stamped in the GET's
drop callback (`RoundTrip.ended_ns`). In the seven runs since, one of them
beside 10,000, the last token told came 0.0–0.1 ms before it. The old sum
was 0.0 and 1.1 ms off it in the two runs that printed both. Steps 1 to
3's "still waiting when the stall ends" read the same sum, and now read
the stamp. The first run cannot be judged again. The stamp says when the
GET was dropped, whatever ended it, so step 10's check also asks that the
round trip end with no error reply. It allows no tolerance, where it
allowed 1 ms: a token told before the final reply is stamped before the
drop, since the subscriber's direct callbacks run on the receive thread
that reads the final reply after them. The check is one-sided: a token
told after the final reply, but before the drop callback ran, would go
unseen.

**`presence_scenarios`**, against `owner-wf` and `zenctl-wf` (main
8880e3c6). §1–§8 ran once, with zk2py's owner and reader. §9 and §10 ran
in every pairing:

| Owner, reader | §1–§8 | §9 | §10 | Not run |
|---|---|---|---|---|
| zk2py, zk2py | 40 | 9 | 46 | 1 |
| Rust, zk2py | – | 9 | 40 | 1 |
| zk2py, zenctl | – | – | 9 | 2 |
| Rust, zenctl | – | – | 8 | 3 |

Every check passes, after the runner's fix. With zk2py's owner and reader,
§10 ran six times. The first failed the one check above, and the five
after the fix passed 46 of 46. Not run:
- §3 step 2, as before;
- §10 step 7 with the Rust owner;
- for zenctl, §9 and §10 step 9.

**Beside 10,000 tokens**, §10 ran whole once with each reader: 46 of 46
with zk2py's, and 9 of 9 with zenctl, step 9 not run.

After review-bot's items on PR #892, §10 ran again with the default
crowd: three times with zk2py's owner and reader, 46 of 46 each, the last
token told 0.1 ms before the drop with no tolerance; and once with zk2py's
owner and zenctl, 9 of 9, unobservable 0.48 s after the cut and ok again
1.49 s after the heal.

§10 step 10 at 0.47, measured on zenoh-python 1.10.1 routers (a release
build), the crowd in `ns`, beside a Rust agent's builds:

| | 2,000 tokens (7 runs; zenctl's 2) | 10,000 tokens (1 run each) |
|---|---|---|
| The round trip at the start | 0.5–45.2 ms, no token | 43.0 ms, no token |
| The first after the reopen | 169–247 ms | 3.94 s |
| The next, asked at once (F-241) | 0.57–3.18 ms | 1.83 ms |
| GETs for the four asks | 1 | 1 |
| Stand-in follower: its seed | 0.13–0.21 s | 3.39 s |
| Stand-in follower: its round trip after the reopen | 78–130 ms | 5.18 s |
| Stand-in follower: complete after the heal | 0.80–2.80 s | 9.85 s |
| Default follower: its seed | 0.13–0.18 s | 3.46 s |
| Default follower: its round trip after the reopen | 65–150 ms | 3.80 s |
| Default follower: complete after the heal | 0.22–2.33 s | 6.49 s |
| zenctl watchdog at `--every 0.5`: unobservable after the cut | 0.45–0.48 s | 0.48 s |
| zenctl watchdog: ok again after the heal | 1.46–1.48 s | 28.5 s |

- zenctl's first transition after the cut was unobservable in every run,
  printed while the link was cut. It never fired, and its stderr held no
  `Unknown interest` and no `Didn't receive DeclareFinal` line.
- Beside 10,000, each follower's round trip after the reopen ran 1.10
  times (the default follower) and 1.53 times (the stand-in) its own seed.
  The stand-in's reads shared the link with the reopen's telling. So a
  wait sized from a seed needs its margin (F-248).
- §10 step 4's watch session in `ns` with history, beside 10,000: its
  round trip took 3.51, 5.87 and 3.70 s, every token heard by its end.
  Step 8's follower handed one stood after 3.09 s (F-251).

**The text, read cold.** 0.47's two changes say what zk2py found. Read
whole, §8.1 and presence §9 and §10 hold three wording points, all minor
(F-250 to F-252). None changes a verdict, a test or an implementation.

**Where the two implementations disagree:** on no verdict.
- **Step 6's round trips** still differ, as 0.43 allows: one before the
  first seed for zk2py, and two for the reference.
- **The time to recover beside 10,000 tokens.** zk2py's follower was
  complete again 6.49 s after the heal, and 9.85 s with its waits at
  20 ms. The debug zenctl took 28.5 s, within the 30 s that step 10 asks
  beside 2,000.

**At 0.49** (round 73). 0.49 is wording only: §8.1's SHOULD on the
interests timeout is about the longest a reader waits (F-250), only the
debug build's 23 s history round trip is past the waits (F-251), and
Expected 10 fails a black box that prints nothing while its link is cut
(F-252). No reader changed, and the runner's step 10 check already gave
0.49's verdict. §9 and §10 ran once with zk2py's owner and reader beside
2,000 tokens: 9 of 9 and 46 of 46.

## A router's admin space beside a router that is declaring (core §4.2 and Appendix B, 0.48)

```bash
PYTHONPATH=impl/python ZK2PY_PROTOC=$(which protoc) \
  target/py-venv/bin/python -m zk2py.state_scenarios [--only 3|10|11|12]... \
  [--reader zk2py|zenctl-doctor|zenctl-storage]... [--rounds 200] [--zenctl <zenctl>]
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.adminspace measure \
  [--cell records+churn|records|0.11+churn|0.48+churn|peer-records+churn]... [--trials 10]
```

Core 0.48 found that zenoh 1.10.1 answers a router's declaration records,
`@/<zid>/router/{subscriber,publisher,queryable,querier,token}/**`, its
`linkstate/*` and its `route/successor/**` while it holds its routing
tables' read lock, so another client's declaration between two replies
freezes it for good. zk2py read it cold in round 73.
- **S4's check** (`live.check_s4`) reads `@/*/router` and
  `@/*/router/status/plugins/storage_manager/storages/**`. 0.11's
  `@/*/router/**/storage_manager/storages/**` reached the records.
- **zk2py asks no selector that reaches them.** `live.admin_guard` refuses
  one, and every GET through `live._answers` passes it.
  - **When.** `_answers` refuses when it is called, before anything is
    asked. Its GET is still made at the first `next()`, as before.
  - **A malformed selector** reaches nothing in the guard. It fails where
    it failed before, at the GET, with zenoh's `ZError`.
  - **Every whatami.** zk2py reads Appendix B's fact as a node's, so a
    peer's or a client's records are refused too (F-253).
  - `live.admin_records_reached` parses a selector as `Session.get` does,
    and uses zenoh's own intersection.
- **The `readings` family** checks 17 selectors and seven calls.
  - S4's two selectors reach nothing, 0.11's reaches six of the seven,
    and `@/*/router/token/zk2/*/*/@zk/**` reaches nothing. `**` crosses
    no verbatim chunk, so a record of a zk2 token is reached by no `**`.
    A GET of such a key reaches neither the admin space nor a queryable
    on `@/**`.
  - The calls: three refusals when called, S4's read asked at the first
    `next()`, a read the user asked for, and two malformed selectors
    failing with zenoh's error.
- **zk2py never ran a hazardous read in a suite.** The admin selectors it
  asks are `@/*/router`, S4's storages read, and `@/<zid>/router` from a
  router process's own session.

**state.md §3 and §12** (`zk2py.state_scenarios`). Both run on a
zenoh-python router in a process of its own (`adminspace.RouterProcess`),
which listens on port 0 and reads its endpoint from its own root document.
Its admin space is on and read-only.
- **The storage.** A zenoh-python router loads no plugin, so the router's
  own session plays the storage manager. It answers
  `@/<zid>/router/status/plugins/storage_manager/storages/plans` with
  `{"key_expr": "zk2/ground/fleet-mgr/**", "volume": "memory"}`. The reply
  carries the router's replier id, so "Who answered" counts it. The
  document is played alone: no data storage answers on the key.
- **The owner** is zk2py's owner of the walkthrough's `mission_plan.v1`.
  Its state queryable ends in `**`, and it holds `plans/vehicle-01`.
- **§3.** The tool's check finds the storage: zk2py's `broken`, and
  `zenctl doctor --check storage-on-state` exits 1 with the finding. The
  router drops the document, and both read clean. The consumer's plan GET
  goes through `Capture`, which records each `get`'s target and
  consolidation and each reply's replier id at the session's API. It asked
  `All` and `Latest`, and the owner alone answered. 5 checks with zenctl,
  3 without.
- **§12.** The rounds run beside a load process: one client holding 200
  subscribers and 200 queryables on `load/<i>/**`, and a second declaring
  and undeclaring a subscriber and a liveliness token flat out. A
  queryable on `@/**`, declared not complete, records every selector it
  hears. Each round is the reader's reads, a put across the router and
  the consumer's S4 GET, each bounded at 2 s.
  - **A missed round ends the run.** The first round whose put or S4
    answer misses its 2 s fails §12 with that round's number, so a frozen
    router fails it at once. Each reader's whole run is bounded at 60 s
    plus 0.5 s a round, and a zenctl read is killed past what is left.
    The router process is killed at the end. With the router stopped
    (SIGSTOP) mid-run, both readers' runs failed at the next round and
    took 19 s together.
  - **The readers.** `zk2py` is `live.check_s4` on one session.
    `zenctl-doctor` is `doctor --check storage-on-state`, which asks the
    same two selectors. `zenctl-storage` is `storage list`, which asks the
    storages selector alone, so Expected 1's first half has nothing to
    judge there.
  - **Expected 3** also counts what the queryable heard against the reads
    the reader makes: a read that is not heard fails. A client's queryable
    never heard the read that froze a router (F-254), so a count is the
    only way Expected 3 can see that read, as 0.50 now says. Expected 2
    fails such a reader as well.

Round 73, on `owner-wk` and `zenctl-wk`, two runs of 200 rounds each:

| Reader | Checks | Slowest put | Slowest S4 answer | Third client's cycles |
|---|---|---|---|---|
| zk2py | 4 of 4 | 0.5–1.0 ms | 1.8–2.8 ms | 8,596–9,216 |
| zenctl doctor's S4 check | 4 of 4 | 1.3–2.6 ms | 2.6–2.9 ms | 28,630–32,144 |
| zenctl storage list | 4 of 4 | 1.1–1.3 ms | 1.8–3.7 ms | 25,529–30,345 |

In each run the queryable on `@/**` heard 400, 400 and 200 GETs. Each
was one of the two selectors, and none reached a record. With every
reader, the whole state runner passes 32 of 32.

Round 74, on `zenctl-wl`, §3 and §12 with the three readers, two runs:
17 of 17 each. The slowest put took 0.3–1.4, 0.5 and 0.5–1.1 ms, and the
slowest S4 answer 1.6–3.1, 1.6–2.2 and 1.7–1.8 ms; the queryable heard
400, 400 and 200 GETs a run, as before.

Round 75 (core 0.51), the same binary, two runs: 17 of 17 each. The
slowest put took 1.7–2.0, 2.9–3.0 and 2.6 ms, and the slowest S4 answer
2.6–3.4, 2.9–4.1 and 1.8–10.2 ms, beside 6,355 to 26,109 of the third
client's cycles; the queryable heard 400, 400 and 200 GETs a run. Its
recorder is a client's queryable: 0.51's Expected 3 says it "may never
hear a read that freezes the router" (F-257), and the runner's count
does not lean on hearing one.

**The hazard, measured** (`zk2py.adminspace measure`, never in a suite).
- **The setup.** Each trial runs one zenoh-python 1.10.1 node in a process
  of its own, the same load beside it, and the `@/**` recorder. A reader
  asks the cell's selectors up to 20 times, with a put across the node
  after each.
- **Frozen** means a put and a GET each failed to cross within 2 s, at
  once and again 12 s later, past the 10 s lease.
- Every GET waits at most 3 s, and the node's process is killed at the
  end, frozen or not.

| Cell | Froze | At ask |
|---|---|---|
| `@/*/router/queryable/**` beside the churn | 9 of 10 | 1, 1, 1, 2, 2, 5, 6, 7, 11; one trial ran all 20 |
| the same, no writer | 0 of 5 | 100 asks, 201 records each |
| 0.11's selector beside the churn | 5 of 5 | 1, 1, 2, 2, 2; up to 401 replies |
| 0.48's two selectors beside the churn | 0 of 5 | 200 asks of each |
| a peer's `@/*/peer/queryable/**` beside its clients' churn | 5 of 5 | 1, 2, 2, 3, 4 |

- **The freezing GET.** It ended at its 3 s timeout, after anywhere from
  0 to 271 of its replies.
- **Afterwards** the node routed nothing again: no put, no GET.
- **Every client kept its link** to the frozen node 12 s on, past zenoh's
  10 s lease. A reader there is not cut off (§8.1). Its reads end at
  their timeouts, possibly incomplete.
- **The `@/**` recorder** heard every GET before the freezing one, and
  never that one, in all 19 freezes (F-254).

## The admin read under access control (core §11.1, §11.2, 0.50 and 0.51)

```bash
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.security_scenarios [--zenctl <zenctl>]
PYTHONPATH=impl/python ZK2PY_PROTOC=$(which protoc) target/py-venv/bin/python -m zk2py.live_interop \
  --only security4 [--zenctl <zenctl>]
PYTHONPATH=impl/python target/py-venv/bin/python -m zk2py.adminspace measure \
  --cell acl-0.15:records+churn --cell acl-0.50:records+churn … [--asks 200]
```

Core 0.50 narrowed §11.1's admin read to the tools' three reads, and
forbade a generator to allow a query that reaches a node's declaration
records. 0.51 scoped that MUST NOT to `deny` and to an ingress `query`,
and said a router under `allow` is not protected (F-256). zk2py read 0.50
cold in round 74, and 0.51 in round 75.
- **The generator** (`acl.generate`) grants the tool `query` on
  `@/*/router`, `@/*/router/status/plugins/storage_manager/storages/**`
  and `@/*/*`, and their `reply`. A block whose allow rules name `query` on
  a key that intersects `live.ADMIN_RECORDS` is refused
  (`HazardousGrant`), unless asked for on purpose (`hazard_ok`).
- **The block check** (`acl.block_hazards`) reads any block, zk2py's or
  `zenctl acl gen`'s JSON5 (`acl.read_block_json5`, through zenoh's own
  loader). It returns:
  - `allowed_query`, under `deny`: an allow rule a policy lists names
    `query` on the ingress flow (named, or zenoh's default of both) and
    on such a key. Since 0.51 an egress `query` alone is not one: "the
    GET that freezes a node is the asker's, decided at the asker's face on
    ingress";
  - `allow_posture`, under `allow`: a policy with no deny of `query` that
    includes `@/**`, whose subjects' GETs reach the records ("a router
    there is not protected", 0.51);
  - `disabled`: access control not enabled.
- **The records' patterns** live in `admin_records.py`, plain strings, so
  `live`'s guard and `acl`'s check share them, and `import zk2py.acl`
  loads no zenoh. Each intersects them with zenoh's own key expressions.
- **The `readings` family** gains 32 cases: the three reads reach no
  record, 0.15's `@/*/router/**` reaches every family, §11.2's inclusion
  pairs by zenoh's own key expressions, the generator's grants and its
  refusal, and the block check on hand-made blocks and on a JSON5 one.
  Under `allow`, only a deny of `query` that includes `@/**` keeps a
  policy off: one on `@/*/router`, `@/*/router/**`, `@/*/**` or `**`
  does not, since `**` crosses no verbatim chunk. Round 75 (0.51) adds
  two cases: a rule on both flows is found once, on ingress, and the
  `allow` block holds no allow rule. An egress-only `query` allow is no
  longer a hazard (469 readings).

**security.md §4** (`zk2py.security_scenarios`, the live runner's
`security4`). R1 is a zenoh-python router in the runner's process,
listening on port 0, its admin space on, read only, under `deny` with the
block under test. Its own session plays `plans`'s storage document and
holds the queryable on `@/**`, since no principal may declare one. The
service principal holds a queryable ending in `**` on its own prefix, so
a GET of the records admitted would be answered. Nothing declares once
step 1 starts. The blocks:
- zk2py's own, for the section's two principals;
- `zenctl acl gen` over `impl/python/interop/security/section4.enrollment.toml`
  (the same two principals: its grants equal zk2py's per principal,
  message and flow, 19 of 19);
- `zenctl acl gen` over `examples/zk2/acl/walkthrough.enrollment.toml`
  (`ground`, which runs the tool `ops`, and `navigation`, among 13
  principals).

Step 4 compiles 0.15's admin read again: zk2py's generator with
`hazard_ok`, zenctl's block with its admin-read rules' keys replaced.
Beside the section, on each block:
- the block check finds no hazard, and finds step 4's `@/*/router/**`;
- with `--zenctl`, once step 3 is over, `zenctl get --no-decode` logs in
  as the tool (a named context in a configuration directory made fresh
  for each call, whose zenoh configuration carries the usrpwd
  credentials) and asks `@/*/router/queryable/**` and `@/*/router/**`,
  each whatever the other did. §4.2 says such a tool "reads nothing of
  them through it, and says the admin space did not answer, never that
  nothing is declared". The check rests on zenctl's exit contract: exit 2
  (no verdict) with no answer printed, and the recorder hearing neither
  GET. The note's text ("no replies to … within 3s") is reported beside
  it, never judged. zenctl-wl's `--timeout` takes whole seconds (it
  refuses `3.0`), so a fractional wait is rounded up;
- under the generator's `allow` block, the tool's GET of R1's queryable
  records is admitted and answered with the service principal's record:
  "a router there is not protected" (0.51).

§11.2's inclusion facts run on a block of their own, each user allowed one
key: under `@/*/router`, `@/*/*` is refused; under the status subtree,
`@/*/router/**` is. Every refused GET ended complete and empty in 0.3–0.7
ms, and the recorder heard none.

Round 74, on `zenctl-wl`: 30 of 30, 9 per block, the generators' grants
compared, and the two inclusion facts. Round 75, on the same binary: 33
of 33, 10 per block with zenctl's `get`. Each refused `get` ran, process
and all, in 12–16 ms. Round 75 was meant as the final cold read of the
admin-space line. It resolved F-256 and F-257 and opened one point,
F-258: §11.1's "The shapes compose" calls the admin read a query on the
routers' answers, where `@/*/*` reaches every node's root document. That
is wording only.

**The freeze under access control** (`adminspace measure`'s `acl-…`
cells, never in a suite). The router process runs zk2py's block for the
tool and the service principal, plus a principal of the measurement's
own, `bench`, which the load, the churn and the probe log in as. The
reader is the tool. The queryable on `@/**` is the router's own session's
(`router --record`), or with `--client-recorder` a client's logged in as
`bench`, allowed to declare it.

| Cell, beside the churn | 0.15's admin read | 0.50's admin read |
|---|---|---|
| `@/*/router/queryable/**` | froze 5 of 5 | 0 of 5, 1,000 GETs refused |
| `@/*/router/**` | froze 5 of 5 | 0 of 5, 1,000 GETs refused |
| 0.11's storages selector | froze 5 of 5 | 0 of 5, 1,000 GETs refused |
| `@/**` | 0 of 5, refused (neither key includes it) | 0 of 5, 1,000 GETs refused |
| the three reads | 0 of 5, `@/*/*` refused | 0 of 5, 3,000 GETs answered |

- **Under 0.15's grant** 10 of the 15 freezes came at the first GET and
  5 at the second. The freezing GET ended at its 3 s timeout, and nothing
  crossed the router again.
- **Under 0.50's** every hazardous GET ended in at most 2 ms with nothing,
  and the recorder heard none of them.
- **The recorder heard the freezing GET** when it sat on the router's own
  session under the block: 20 of 20 freezes. On a client under the same
  block it heard it in 0 of 4, and on the router's own session without a
  block in 0 of 7 (F-257).

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
    (0.10): zk2py counts no budget and runs no archive;
  - `desired.v1`'s scenario §9, a tool's (`zenctl desired`, #840's DD):
    zk2py's §2.17 runs on fixtures only.
  - a storage manager: a zenoh-python router loads no plugin. state.md §3
    and §12 play a storage's document on the router's own session, so
    §4.2's statement (0.48) that every zenoh 1.x storage manager answers
    under `status/plugins/`, never under `config/`, is not measured here.

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
    owner.py          §3.3 §4 §5 §6 §8 a minimal owner, a standby (0.39), member tokens (§8.1)
    live_interop.py             the live runner, with the Rust owner and consume examples
    acl.py            §11       the grant generator: a deployment → zenoh's access_control block
    acl_interop.py    §11       the access-control run (`--only acl`)
    security_scenarios.py §11.1 §11.2 scenarios/security.md §4 (0.50, 0.51) on zk2py's block and zenctl acl gen's,
                                zenctl get as the tool (§4.2), and §11.2's inclusion facts (`--only security4`)
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
    telemetry.py      telemetry.v1 the standard schema's ids, declarations, carriers, a sample's conformance,
                                a pair's epoch, continuity and rate, a contract's and a bundle's resources
    telemetry_scenarios.py telemetry.v1 scenarios.md §1–§9 against zk2py's owner or the Rust owner example, §8 across a face too,
                                and zenctl telemetry judging beside zk2py's reader
    desired_live.py   desired.v1 the author and the target on a bus, the scenarios' drive
    desired_owner.py  desired.v1 either party as a process, driven as the owner example's --desired-* are
    desired_scenarios.py desired.v1 scenarios.md §1–§10, edges, a re-mint and the tool's shapes, with any two
                                parties, zk2py's tool and zenctl's
    desired_tool.py   desired.v1 §2.17's tool on a bus: the slice, the report, the author's version, a window
    desired.py        desired.v1 the standard schema's ids, declarations, bindings and the slice's key, stamps, equal
                                documents, the target's state machine, the report's reader, §2.17's conclusions
    budget.py         §2.7      the population budget: the bound, a population, a rate, lossless, the stall watch, readings on a bus
    budget_scenarios.py §2.7    scenarios/budget.md §1–§7 and §1's stalled reader: the measurements, and the owner keeping its budget
    seed.py           §4.2      a late joiner's seed: subscribe, then S4's GET, merged by stamp; the pure merge
    presence.py       §8.1      what a complete presence read is (0.37, 0.41): the view (a withdrawal unconfirmed until a
                                read, a read unordered while no round trip stands, a transport's move) and the follower
                                on a watch session
    presence_scenarios.py §8.1  scenarios/presence.md §1–§10 (0.41) with either owner and either reader under test
    redundancy.py     redundancy.v1 the declaration, an entry's mark, the election, the heal, the state machine,
                                what a runtime gives back, §5's questions
    redundancy_live.py redundancy.v1 a candidate on a bus, the following consumer, a tool's two reads
    redundancy_owner.py redundancy.v1 one candidate as a process
    redundancy_scenarios.py redundancy.v1 scenarios.md §1–§12 and a re-mint, zk2py against itself
    timing.py         timing.v1 the declaration, the deadline tracker and a tool's held time, the lifespan
                                judgement, the period window, §5's conclusions and roll-up, the profile's linter
    timing_live.py    timing.v1 a consumer's deadline events and lifespan discard on a bus, a tool's window,
                                an owner's offset stamping clock
    timing_scenarios.py timing.v1 scenarios.md §1–§6, zk2py only, and core 0.53's HLC facts measured
    timing_tool.py    timing.v1 the tool (§2.7, §5) as a process: one owner's timed streams, the window,
                                each member's answers, each resource's and the run's, across a face too
    timing_interop.py timing.v1 scenarios.md §1–§5 in every pairing with the Rust owner and reader (black
                                boxes), §6's six steps, §2.8 and §2.7's windows with both tools, zk2py's and
                                `zenctl timing`, beside each owner
    arbitration.py    arbitration.v1 the declaration, the ranking, the arbiter (§2.5–§2.11), §5's conclusions,
                                the linter
    arbitration_live.py arbitration.v1 an actuator's runtime on a bus (a governed interface's incumbent
                                followed), a commander (a redundancy.v1 candidate too), a tool's own view
    arbitration_owner.py arbitration.v1 a commander or an actuator as a process, its switches as JSON lines,
                                the reference owner's commander and arbitrate words too
    arbitration_tool.py arbitration.v1 the tool (§2.14, §5) as a process: a consumer's roles, the revisions held
                                and retrieved, the tool's own view, §5's three questions
    arbitration_scenarios.py arbitration.v1 scenarios.md §1–§11 in every pairing with the Rust commanders and
                                actuators (black boxes), §10 with both tools, zk2py's and `zenctl arbitration`,
                                F-268's control, core 0.56's open measured
    state_scenarios.py §4.2     scenarios/state.md §3 and §12 on a router process, §10 against zk2py's owner,
                                and §11's open cases
    admin_records.py  §4.2 §11.1 the keys a node answers under its tables' read lock, shared by live and acl
    adminspace.py     §4.2 App. B a router or a peer in a process of its own, the load beside it, the @/** recorder,
                                and the freeze measurement (never a suite), under access control too (0.50)
    proc.py                     helper processes for the measurements: a router, a publisher, a consumer that stops
    checks.py                   a live runner's checks: known failures by identity, XFAIL and XPASS, the exit
  interop/            zk2py's own interop contracts: probe, echo, needs, bringup, tc, scan, sysinfo,
                      sysinfo_x, tracker, order, order_b, budget, plans, epoch (presence.md §3); freshness/ holds beacon.v1 and zk2py_fresh.v1, budget/ budget.v1 and zk2py_stall.v1; config/ zk2py_author.v1;
                      telemetry/ meter.v1 and meter.json, telemetry.v1's scenarios' contract, and meterx.v1,
                      zk2py's own, a declaration invalid for §5's over many samples;
                      rev/ holds bringup's minor 1; stand-in/ an archive.v1 id (§4.4);
                      redundancy/ ledger.v1 and valve.v1, redundancy.v1's scenarios' contracts, and
                      ledger-vote.v1, §11's invalid election; security/ section4.enrollment.toml,
                      security.md §4's two principals for `zenctl acl gen`; timing/ pace.v1,
                      timing.v1's scenarios' contract; arbitration/ steer.v1 and rudder.v1,
                      arbitration.v1's scenarios' contracts
```

The JSON schemas are read from `spec/` at run time (`shape.py`), not copied.
So the E000 and D000 checks follow the published schema files exactly.
