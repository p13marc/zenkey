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
| contracts | §9.1–§9.5, App. D | `contract`, `schemas`, `shape`, `protoc`, `jcs` | the exact sorted codes, the canonical JCS bytes and the fingerprint of every fixture. The protobuf well-known types, the five `e000-toml11-*` cases (0.4) and `cardinality = "unbounded"` (0.57, 4294967295 from resolution on) are included. |
| sets | §9.2 | `sets` | E035, E036 |
| bundles | §9.6 | `bundle` | the twelve verification steps and their tags. `seed.toml` also **builds** to `valid.bundle.json` byte for byte, and `contracts/ok-packed.toml` to `packed.bundle.json` (0.75). |
| history | §9.7 | `history` | `[at, tag]` per history root |
| descriptors | §3.3 | `descriptor` | the D codes (D012, a standby marked tokenless, since 0.39), against `descriptors/contracts/nav.v2.toml`. That contract's fingerprint, computed here, is the `sha256:fea2…` the fixtures expect. |
| errors | §5.2, §7.5 | `zk2c` | `forms.json`: the error form's 256 bytes, its writes and its reads by Encoding, an earlier core's three envelopes `encoding`. `cases.json`, the bare wire's envelope, went with core 0.99 (round 104). An unknown file there fails. |
| envelope | §7.5, §5.1, §5.2 (0.72 to 0.94) | `zk2c`, `calls` | `vectors.json` (each header and case, accepted ones written back), `carries.json`, and `calls.json` (round 101): an operation's forms by its types' tags, its writes and its reads, a packed record under CH's tag |
| budget | core §2.7, observer §5 (0.24, 0.25, 0.27) | `budget` | every file of `budget/`: `bounds.json` (the bound across instances), `population.json` (a GET's or a window's population against it), `rate.json` (an event's rate per member) and `lossless.json` (whether a window is shown to have lost no delivery, 0.25; its `reader_stall` and `stall_bounds`, 0.27). An unknown file there fails. |
| seed | §4.2 (0.27, 0.100) | `seed`, `seed_snapshot` | `seed/merge.json`: `order` (two stamps, or a stamp and none, compared) and `cases` (a key's arrivals, each kept or not); `inband.json`; and `snapshot.json` (#977 E2, round 104; core 0.100's §4.2 since round 105), a seed over snapshots: 45 cases, F-497's three mapping the index by the revision the owner's descriptor names. An unknown file there fails. |
| compat | §9.7, §9.8 | `compat` | **all 100 cases** (spec 0.8), evaluated through `compat/README.md`'s one-resource wrapper: §9.8's six tables, the JSON Schema and protobuf rules, `same_revision`, and the FULL_TRANSITIVE cases (with each pairwise `against`). The packed cases (0.75) are wrapped as `packed` artifacts. |
| packed | §7.6, §7.5 (0.75, 0.76) | `packed`, `zk2c` | `packed/vectors.json`: each layout's form, sha256, tag, items and bit bounds; each write's bits, pad and bytes, or the writer's refusal; each read's JSON reading, or the first refusal in §7.6's order. Every accepted write is sealed in the envelope, opened and read back; every read is made through the envelope too. Live against the reference: `python -m zk2py.packed_live` (the mend after round 94). |
| hostid | profiles/hostid §2.1, §2.11 | `hostid` | every file of `spec/profiles/hostid/conformance/`: `vectors.json` (the derivation) and `shapes.json` (the minted shape). An unknown file there fails (`profiles/README.md`). |
| freshness | profiles/freshness §2.1–§2.8 | `freshness` | every file of `spec/profiles/freshness/conformance/`: `horizons.json` (a resource's horizon), `judgements.json` (one member's combined verdict, seconds read exactly to the nanosecond) and `clock-trust.json` (a measured clock's trust over one reading, 0.2). An unknown file there fails. |
| health | profiles/health §2.2, §2.10, §2.11 | `health` | every file of `spec/profiles/health/conformance/`: `judgements.json` (§2.11's procedure over one reading), `rollups.json` (a tool's roll-up), `codes.json` (fault codes) and `clock.json` (§5's clock question, 0.3). An unknown file there fails. |
| config | profiles/config §2.2–§2.19, `v2.md` §2–§3 | `config`, `config_v2` | every file of `spec/profiles/config/conformance/`: `validation.json` (§2.5's, §2.10's and §2.13's orders of refusals), `window.json` (§2.8's window and deadline), `lifecycle.json` (the owner as a state machine: each step's reply, device acts, events and state), `desired.json` (§2.18) and `redaction.json` (§2.15); and config.v2's (round 102): `v2/`'s six files, `derive.json` (the declaration, the derived types and their layouts, the contract's lint), `records.json`, `validation.json`, `window.json`, `lifecycle.json` and `redaction.json`, and `contracts/rfcfg.v1`, lint-clean. An unknown file there fails. |
| snapshot | §7.5 (0.97) | `snapshot`, `zk2c` | every file of `snapshot/`: `bodies.json`'s `writes` (each page's bytes and size, or the writer's refusal, read back to its value), `reads` (each page, written back, or the first refusal in the reading's order) and `pages` (each snapshot's pages and sizes, or the refusal, read back complete, and cut a second way by arithmetic). An unknown file there fails. |
| frame | §7.7, observer §7.6 | `frame` | every file of `frame/`: `layouts.json` (the frame form, its tag, the widths and the span), `bodies.json` (each write's bits, pad and bytes or its refusal, each read's value or its first refusal, every accepted one written back and sealed in form 5) and `apply.json` (round 97: one key's frames through `frame.Reader`, from each body's value and as a sealed put, envelope first). An unknown file there fails. |
| link | profiles/link §2.1–§2.15 | `link` | every file of `spec/profiles/link/conformance/`, `contracts/` (0.13: each contract there valid, each artifact listed) among them: `exposure.json` (§2.2's exposure and whether a resource's pushes cross), `plan.json` (a face policy and a deployment to each face's plan, or its refusal) and `lints.json` (the policy's linter, the plan's warnings, the windows and a router's block). An unknown file there fails. |
| alarms | profiles/alarms §2.2–§2.6, §2.12, §2.14, §2.15, §5 | `alarms` | every file of `spec/profiles/alarms/conformance/`: `keys.json` (the recipe through the sanitizing step, the strict recipe beside it, host-scoped names, references and their keys), `documents.json` (readable, the owner's errors and warnings; a case given as its `text` read as strict JSON first, 0.2), `transitions.json` (one key's samples read in stamp order) and `conclusions.json` (§5's four questions, and §2.15's verdict on an owner's alarms). An unknown file there fails. |
| telemetry | profiles/telemetry §2.1–§2.11, §3 | `telemetry` | every file of `spec/profiles/telemetry/conformance/`: `schema.json` (the standard schema's published ids, the file's header against zk2py's tables, and each case's document by path, inside a bundle, or inline), `declarations.json` (a resolved resource's declaration), `samples.json` (one sample's conformance, and the member's kind and unit; since 0.2 a sample may come as its bytes, `json_text` read as strict JSON, `cbor_hex` as CBOR, `attachment_text` as JSON) and `continuity.json` (a pair's continuity, its epoch's evidence and its rate, compared exactly; `stamp_ntp64` read as zenoh's `as_nanos`, 0.2). An unknown file there fails. |
| desired | profiles/desired §2.2–§2.10, §2.17, §3 | `desired` | every file of `spec/profiles/desired/conformance/`: `schema.json` (the standard schema's two published ids, the file's header against zk2py's tables, each case's document by path or inline), `declarations.json` (a resolved resource with its parameters' types -> its declaration, or its reasons in order, `not_string` since 0.2), `bindings.json` (a requirement as a descriptor lists it -> the slice's key, the kill switch, or its findings), `order.json` (one role's steps through the target's state machine, a put's payload decoded, as its text or as its bytes: each step's word and the report after the last, which must itself read with no finding), `reports.json` (a report decoded or as its bytes: readable, and its findings in order), `conclusions.json` (§5's four questions over one reading, the deployment's word across a face since 0.2) and `slices.json` (0.5: a tool's reading of one role before §2.17 answers -> its candidate slices, where they came from, §2.3's and §2.2's problems, and which reply is the report). An unknown file there fails. |
| redundancy | profiles/redundancy §2.2–§2.15, §5 | `redundancy` | every file of `spec/profiles/redundancy/conformance/`: `declarations.json` (a contract's `uses` and resolved annotations, and the tokenless set -> governed with its election, not governed, or its reasons in order), `descriptors.json` (one entry's mark and §2.14's findings, `malformed` alone), `elections.json` (one read from the instance's view -> take over, or stand by and why), `heals.json` (two reads' holders -> step down to the lower, or stay and why), `lifecycle.json` (one candidate's events through the state machine -> each step and its role) and `conclusions.json` (§5's four questions over two reads a grace apart). An unknown file there fails. |
| timing | profiles/timing §2.2–§2.7, §5 | `timing` | every file of `spec/profiles/timing/conformance/`: `declarations.json` (a resolved resource, its annotations or its three layers, or a requirement -> not this profile's, ignored, invalid with its reasons in order, or declared with `below_period`), `deadlines.json` (one reader's tracker for one provider and resource, driven by its events, each instant read exactly to the nanosecond -> its reports, what it holds, its next due), `lifespans.json` (one member's samples within one reading -> each sample's judgement, a clock falsified for the reading), `periods.json` (one member's arrivals over a window -> kept, not kept, too short or not heard, with its count and span) and `conclusions.json` (§5's three questions of one member over one window). An unknown file there fails. |
| arbitration | profiles/arbitration §2.2–§2.11, §2.14, §5 | `arbitration` | every file of `spec/profiles/arbitration/conformance/`: `declarations.json` (one role and the revision of its required interface held -> not arbitrated, arbitrated with its policy, stream, deadline and period, unread, or invalid with its reasons in order), `rankings.json` (a binding and the addresses heard -> the providers in rank order, the unbound, the wildcard and shadowed entries), `arbitrations.json` (one consumer's role and its events -> after each, its switches, the active provider, its due, a sample's outcome, the reads again) and `conclusions.json` (§5's three questions over a tool's reading). An unknown file there fails. |
| readings | | `alarms`, `telemetry`, `desired`, `redundancy`, `timing`, `arbitration`, `presence`, `live` | not a fixture family: zk2py's own cases, each naming what it reads (an open finding's reading, or a stated rule beyond its fixture). Today, 768. Of the mend after round 93, 9: each text at its version, F-414 to F-421 answered as they read and the forms they replaced gone (§8.2's stamp a session adds, §3.3's descriptor put, §4.3's record and random run, "its seq for state and events" in §4.3, S7 and `RULES.md`, `types.md` §1, `desired.v1` scenarios §6), `Owner.take_run` over fourteen run records (absent, good, and twelve it cannot use: a 32-bit draw, the record left as it is), three draws with no record, and `zk2c.read_put` and `read_delete` over 3,849 payloads, 1,500 random, every CH byte with up to 8 more, and every truncation of four envelopes: none raises, each reading plain, refused with its token, or accepted and written back to the same bytes (review-bot on PR #1005). Of round 93, 7: each text at its version, §4.3's table read from a contract, which keys carry the envelope, a refused envelope's reading and its echo row, a late joiner merging no refused envelope, and S1's check off the table. Of the mend after #926 part 5, 15: each text at its version, F-406 to F-413 answered as they read and the forms they replaced gone, what a tool says of a payload whose declared type it does not hold (four cases, each with where it stopped resolving the key), the echo tool's rows and notes, sniffing over eight payloads, the decode order's wire and decoding (types.md §2's `{` read as CBOR), config.v1's reader by its sample's Encoding, a document that is not an object undecodable and named by every answer (review-bot on PR #994), a profile's reader by its profile, each forgetting counted, and state.md §7 step 2's premise. Of rounds 91 and 92, 11 and 9, each in its section below. Of round 90, 11: each text at its version, F-358 to F-378's answers as they read and the forms they replaced gone (§12's full level, a static section's role rule, how a static rule binds, the rules without a keyword, F-363's two restatements, the three rules core 0.67 changed and `observer.md` 0.9's two sentences, `RULES.md`'s S7 row, a late joiner under observer, §9.4's E032, a pronoun's noun, and the glossary's corrected terms), and S7's consumer rule after a late joiner's seed: `seed.follows` over seven pairs (a new id's lower time kept, one id by time, equal stamps once, unstamped below stamped, an old id back after the new one kept), `seed.ordered` over a seed then what follows it, and a `LateJoiner` on a fake session, a consumer's ordering by S7 once its GET ended and a tool's by §4.2's merge throughout, and the switch decided by arrival, a sample and the GET's final reply interleaved both ways, in one thread and in two (review-bot on PR #969). The sentences round 90's findings ask #926 part 3 to reword are read by a phrase their fixes keep, never word for word. Of round 89, 20: each text at its version, F-347 to F-357's answers as they read, the 22 sentences core 0.66 and `observer.md` 0.8 made name their role (and their old forms gone), §12's and observer §0's new text, zk2py's walk of `spec/RULES.md` (`zk2py.classes`), whose verdicts each name a rule the texts still state, its rows with no verdict and its counts reported below the totals, the glossary's terms each linked, `zk2py_replica.v1` lint-clean with the reference CLI's fingerprint, and seven of the owner rules the walk found unkept and zk2py keeps now (§2.4's QoS on a member's publisher and an event's put, §3.3's cardinality, O6's one summary and what a handler that raises sends, §5.2's detail, §5.1's "Beside replicas", observer §7.4 at the echo tool's start), on a session that records what is declared. Of round 88, 13 (one after review-bot on PR #963): each text at its version or later, `desired.v1` 0.10's and `config.v1` 0.8's sentences for F-336 and the rewrites of F-337 to F-346 as they read, §12's two contracts (`configs.v1`, `zk2py_configured.v1`) lint-clean with the reference CLI's fingerprints, `Core.desire` and `Core.revert` saying how an act ended (a delete's revert one set back to the startup value, no `desired` after it), the bound version acted on again after a restart and the same version within a run, and the reference's figures `arbitration.v1` 0.4 moved into sentences of their own (a read 0.1 s after a withdrawal, a first put 500 ms after the tokens) held by zk2py's actuator and commander; and the fed owner's role left free for the next version by a refused document and by an act that raises (failed, `apply`, counted). Of round 87, 9: observer 0.7's §5.2 and §1, core 0.63's "Not measured" line, `alarms.v1`'s §2.15 at 0.9 or later, and three of `desired.v1` 0.9's split rules read by `TargetRole`. Of `observer.md` 0.6, 7 (round 86; one after review-bot on PR #959): §5.2's watch after a window as the text states it, `alarms.v1` §2.15's and its scenarios' §10's citations of it, and zk2py's alarms reader ending its window by the budget windows' watch, the longer of the lease and `wait_before_close` plus the reply timeout (four lengths, five sets of the three, and `AlarmsReader.window` itself at three, on a reader built with no session by `AlarmsReader.offline`, whose losses, withdrawals and GETs it reads too). Of `observer.md` 0.5, 12 (round 85; four after review-bot on PR #957): §1's NotAsked that spells not this profile's (F-295), over the 15 such answers of the three fixtures the rule cites (6, 1 and 8), no cell a bare not asked and each saying not this profile's or naming its reason, each field keeping `not_asked` with its reasons, each note naming both; zk2py's freshness tool's member field keeping each member's reason, a deleted member's cell not this profile's; the member field's boundary (`service_members` reading it, `member_verdicts` refusing a bare token and three other shapes, `member_fields` refusing a token string); and §5.2's watch after a window, the longer of the lease and `wait_before_close` plus the reply timeout, a caller's own lease too (F-320's reading). Of `observer.md` 0.3, as 0.4 states it, 36 (round 84; five after review-bot on PR #955: the NDJSON rows queued by zenoh's callback and written by the main thread, a failed write the stream cut, and §1's two lists covering the six fixtures): §1's two spellings of not this profile's read over every answer of the six fixtures §1 cites (the token 11, 9 and 5 times in `timing.v1`'s, `arbitration.v1`'s and `redundancy.v1`'s; NotAsked with a premise reason 6, 1 and 8 times in `freshness.v1`'s, `telemetry.v1`'s and `desired.v1`'s; every other `not_asked` NotAsked with its reason), a premise reason binding only the profile that names it, nine single readings, the three media keeping it apart (a NotAsked of the second spelling keeping its reason in the cell, F-295), and §8.2's medium: the echo tool's NDJSON row a sample's alone, its stderr line stating every kind excluded. Of `observer.md` 0.2 and core 0.59, 28 (round 83), 22 of its rules, two of the steps' contracts and four of F-291: §3.2's transient half over three reads a grace apart, a shape in the first alone no fault (operations.md §8 step 6), in two consecutive reads a fault, and in the first and the last but not between none, one read or none refused as no pair (review-bot on PR #952); §7.2's two halves in operations.md §5 step 2's words (told, the O3 finding with an access-control refusal said to be silent too; not told, unobservable, the premise named), never "no such operation"; §7.4's report (any pool, none below 2 MiB, the limit less 2 MiB, an unreadable limit leaving the check unobservable); §8.2's scope as zk2py's echo tool states it over seven selectors (grammar.md §1 step 5); the steps' two contracts lint-clean with the reference CLI's fingerprints; and four of F-291 (resolved by 0.3), the three ways the profiles' fixtures give "not this profile's". Of core 0.57, 13 (round 82): `"unbounded"` on a stream's and an operation's template lint-clean and resolved to 4294967295, one fingerprint with the number; on a template without parameters E014; four other strings E000; the bound no ceiling, lowered to 64 by an instance; D007 admitting a stated 4294967295 and refusing 4294967296; the two spellings one revision. Of `observer.md` 0.1, 48: 32 selectors' verbatim blind spots (§8.2), each kind reached by a selector of its key form (core §1.1, §2.5) and missed by one shorter and one longer, `**` before `@adv` spanning chunks and a selector's `**` crossing none; a timing tool's timed streams read from the contract, and its reach under six selectors, two naming another address; and eight `RLIMIT_MEMLOCK` notes, core §7.4's words quoted, a limit that cannot be read said so (§7.4). Of `timing.v1` 0.5, 6 (round 81): a known member missed with its owner present, then its owner gone, held to the window's end while it stays missed, its deadline `no`; one paused by its owner's absence, then heard while its owner is absent, a new stretch from that arrival, 2,000 ms, `met`; a member ended in the window and not heard again keeping its held time, its first end, `ended`; a run's roll-up handed its members in row order reading its reason in byte order (F-279); and, after review-bot on PR #941, the roll-up's two forms explicit (`rollup` takes `(key, Answer)` pairs alone, `rollup_answer` answer tokens alone, each refusing the other's and a mix) and a tool's window closing once (a second `end` adds no report). F-278's and F-280's readings went: `deadlines.json` #46 to #51, `periods.json` #21 and `conclusions.json` #50 hold them. Of `timing.v1` 0.4, 1: where no tracker is held, an end before the member was first held in the window ending nothing (review-bot on PR #939). Of `arbitration.v1` 0.3, 3: a view governed only when every revision held lists `redundancy.v1`, listening counted from the last subscription, and a view 100 ms after its subscriptions, its watch's 4 s round trip before them, `listening` and never `dead_man`. Of `timing.v1` 0.3, 5: a tool's known members (templates without parameters), a loss it cannot attribute counted for every member, the `meta.zid` across a face on the deployment's word alone and away from one the descriptors', and F-277's roll-up, since text 0.4 (round 80) read as 0.4 writes it: every member set aside not asked, with the first not-asked member's reason, or not this profile's when every one is, none at all unobservable, with no reason since 0.5 (F-279), and a set-aside member beside one pole changing nothing. F-275's reading went: `deadlines.json` holds it since 0.3. Of `arbitration.v1` 0.2, 2: the counts line every provider known in rank order, and §11's governed `steer.v1` minor 1 loading with the reference CLI's fingerprint, `rudder.v1` arbitrated over it. Of `timing.v1` 0.2, 5: F-261's two orders through an epoch tracker (a read before any member token seen ending nothing, the wait from the token seen), F-260's `beyond_bound` once per stay past the bound, F-259's trackers, and F-275's epoch member waited for through its owner's absence. Of `arbitration.v1` 0.1, 13: the walkthrough's `thruster.v1` over `twist_cmd.v1` arbitrated and lint-clean, its enrollment's two bindings exact, the scenarios' `rudder.v1` over `steer.v1`, the linter's checks with and without the required interface, and F-268's two orders through the arbiter; and six from review-bot on PR #912: a tick just past the due moving the state in every state `arbitrations.json` reaches, the round trip after a move asked until one stands (a later move ending it, a cut-off waiting for a link), and a binding entry that is not one refused. Of `timing.v1` 0.1, 17: the adopters (`twist_cmd.v1`'s `cmd`, `nav.v2`'s `position`, `pace.v1`'s two streams) declaring as the text reads them, each lint-clean; the profile's linter's four warnings; §5's roll-up by each question's finding beside the sentence as written (F-264); a missed member staying missed when its owner is present again (F-262); a known epoch member ended by a read before its member token, as 0.1 read it (F-261; its read after the token is 0.2's now); a `meta.zid` of 33 digits stating none. Of core 0.42's §8.1, 15: a watch session in no namespace on the wire keys (the wire selector and its round trip's key, a namespace with a slash at either end read as its chunks on the wire and stripped alike, each key heard stripped, one outside the namespace not the deployment's, `Stripped` counting it and handing on the rest, `watch_or_beside` in a namespace declaring on the wire key and making no GET on the reading session, the caller's drop hook and callback mode kept), zk2py's follower over fake sessions (its watch session cut off: no round trip and no read asked on either session, the read not made with the watch session's reason, what it would establish unobservable; linked: the round trip then an ordered read; the watch session's links gone at the read's end: unordered; in a namespace, without history on the wire key, keys stripped and one outside counted, never folded; handed a namespaced watch session, 0.41's history), and a read or a round trip on a session whose interests timeout zk2py cannot read saying its completeness rests on zenoh's 10 s. Of core 0.41's §8.1, 28: a read whose time reached its session's interests timeout possibly incomplete, saying so, and what zk2py records of a session (its interests timeout, 10 s for one it did not open; its namespace); the round trip's key under the subscribers' subtree, and when it stands (its final reply, no error reply, before the interests timeout, a link before and after; none made through a session cut off, nor one whose links cannot be read); the view's unordered reads (what they show up, what they do not unobservable, never down, a withdrawal keeping its reason, an ordered read owed after any of them) and a transport's move (what the reader held unobservable, a read owed, a withdrawal keeping its reason). Of `desired.v1` 0.8, 4 of a revision that does not read beside an unread requirement (no resources list shows nothing: unread, never `not_a_target`; a readable held revision declaring no desired resource beside one that does not read, `not_a_target`; read, a revision that does not read is not in hand, so the target alone). Of `redundancy.v1` 0.2, 5 of the kept events decided partway (F-199: a read deciding take_over keeps the appearances through the act and drops the withdrawal, one deciding step_down the reverse, a heal read ending the heal with three kept re-entering the rest in `heal_first`, and over every state reached under both elections each step re-entered asking nothing). Of `desired.v1` 0.4's §2.17, 6 of how long behind (F-211: every reading over the window, another answer or version starting it again, one reading enough only on a trusted clock). Of `desired.v1` 0.4's tool, 9: §2.10's choice of the report among a target's replies (the readable one claiming the slice, two differing unreadable, one twice one, an unreadable one beside none readable claiming it, a readable one beside an unreadable, none claiming), and §2.17's slice from the target alone (the requirement's template and binding, the kind token from the report's key, a parameter left unbound). Of core 0.38's §8.1, 3 more of `list_presence`'s guards: the check reads the links, a closing transport still listed notwithstanding, and says a read was cut off while it ran. Of desired.v1 0.3's live half, 20: zk2py's author's encoding (nine numbers written by their exact values and read back the same, members sorted, a non-finite number refused) and nine of §2.4's revision read exactly. Of core 0.37's §8.1, 28: zk2py's presence view (a withdrawal unobservable and owing a read, down only from a complete read, up again from one, unconfirmed still after a read not made or possibly incomplete, a re-mint owing none, one read for a burst, a reply sent before a withdrawal not bringing it back, each key's latest move since a read was asked deciding it, a withdrawal heard back owing no read, an address never heard of down or unobservable by the latest read over it, the three holders), a read's three kinds, and `list_presence`'s guards on fake sessions (a transports accessor that fails, before or after, leaves a read possibly incomplete; the registry prunes closed sessions, never hands an entry to a session with a reused id, and closes a closed reader's watch session; since 0.38 the accessor is the links). Of `desired.v1`, 73: Appendix B over the adopters' real contracts and bindings (the examples' eight desired resources, the walkthrough executor's slice, `zs.desired.v1`'s seven templates, `zs.thresholds.v1`'s requirement bound for the sysinfo sensor and unbound, its `applied/{topic}` no Report), the scenarios' slice `D`, zk2py's own refusal of a target address that is not `<system>/<service>` (five), zk2py's readings of round 52's F-162, F-163, F-166, F-167 and F-168, now 0.2's rules (the 16 ids remembered and the 17th forgetting the first, a restart forgetting every id, exact values from a payload's text, 256 and 257 deep), of the open F-181 (a report's `18446744073709551615.0` read as its double) and F-184 (an owner's waiting version re-delivered is the same version), §2.7's equality over values strict JSON cannot yield and over exponents of 10^9, 10^23 and 100,000 digits, each finishing at once, finiteness at the edge 2^1024 − 2^970 written four ways, §2.10's report puts, and a report's bytes no fixture gives (nested 100,000 and 300 deep, a byte-order mark, a lone surrogate). Of `telemetry.v1`, 66: Appendix B over the adopters' real contracts (zenoh-modem's 73 declared streams and 23 units, `sdu/dropped_total` a bare counter with no epoch, each ZenSight contract's revision 1.1 and both published bundles a Point declaring no kind, netring's two units, tcgui's resources none this profile's), three of §2.3's copies (a JCS-equal one keeps the id; a changed one is not the Point), two of F-148's state order, 14 of F-149's `null` members (a Point's `value`, each kind's Value, each Histogram member, and `epoch`, `unit` and `labels`), and four of 0.2's open F-150 (the double's edge, read as IEEE 754 rounds it) and F-151 (an Epoch attachment's lone surrogate, read strictly), three samples nested 100,000 deep, read as undecodable, never a crash, sixteen of §5's over many pairs and samples (`verdict`, per member since 0.4, over unobservable samples since 0.5), F-152's reading and F-157's among them, five of CBOR tag chains (an outer tag around a converted bignum, 100,000 tags), and three of §2.9's seed folded into a subscription's moves, the subscription's word winning. Of `alarms.v1`, 137: 11 of `alarms.v1` §2.4's whitespace (RFC 8259's `ws` around a payload's value reads; anything else after it, and a byte-order mark before it, is `json`), six of §2.15's window healed only by a GET asked after a break was seen, two of 0.5's whole scope (an archive's read plays no part), 86 of §2.16's change reporter (over every `transitions.json` case, every `conclusions.json` reading, four loss sequences, four of 0.6's first answers, and four of a key gone from the scope), five of 0.6's presence read dated by its ask, six of §2.14 step 5's left-out key, a `reply_del` among them, and 21 of §2.16's two closed lists: each of the twenty neither reasons the first question gives, read from a reading of its own, on its list, and the table's reader's-sight reasons 0.8's ten. |
| examples | §9.6–§9.8, §10 | | Every `examples/zk2/**/<name>.v<major>.toml`, and every profile's standard contract under `spec/profiles/` (0.23): loads with **no finding at all**, W107 included; its built bundle verifies; it is published in its tree's history root (`examples/zk2/.history` or `spec/profiles/.history`), **byte-identical** to the bundle zk2py builds; it is `compatible` with its history. Both roots pass the §9.7 check, and a revision held in both is one file, byte for byte. |

The result at the time of writing:

```text
layout         25 passed     0 failed
keys           55 passed     0 failed
slugs          42 passed     0 failed
templates      11 passed     0 failed
contracts     112 passed     0 failed
sets            4 passed     0 failed
bundles        24 passed     0 failed
history        10 passed     0 failed
descriptors    54 passed     0 failed
errors         45 passed     0 failed
compat        100 passed     0 failed
budget         95 passed     0 failed
seed           67 passed     0 failed
envelope      108 passed     0 failed
examples      108 passed     0 failed
readings      768 passed     0 failed
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
total        3887 passed     0 failed
```

The figures are against `core.md` 0.74, `observer.md` 0.15, `hostid.v1`
0.9, `freshness.v1` 0.5, `health.v1` 0.7, `config.v1` 0.8, `link.v1` 0.10,
`alarms.v1` 0.10, `telemetry.v1` 0.7, `desired.v1` 0.13, `redundancy.v1`
0.9, `timing.v1` 0.8 and `arbitration.v1` 0.5, and `spec/RULES.md` as
generated at dcd5f968 (the mend after round 93).
Core 0.74 and `desired.v1` 0.13 changed no fixture, and no rule's keyword:
they settle the scope of §8.2's and §4.3's rules and reword two
(F-414 to F-421). `readings` went to 768: nine of the mend, in
`_mend_after_round93_readings` (round 93's seven went to 759).
Core 0.71 to 0.73 and `observer.md` 0.13 to 0.15 added the `envelope`
family (108) and grew `seed` (67) and `descriptors` (54).
Core 0.67 and `observer.md` 0.9 changed no fixture. They changed four
rules, §2.4's reliability, S7's seed, §1.3's guard and observer §7.3's
decoding, and restated two (§2.2, §11.2). Of the four, S7's moved zk2py:
its late joiner orders what a consumer receives after its seed by S7's
consumer rule (round 90). `readings` went to 717: eleven of round 90, in
`_round90_readings` (round 89's twenty went to 706).
Core 0.65 and 0.66, `observer.md` 0.8 and #935's sixth part changed no
fixture, and no rule but `desired.v1` 0.11's §2.5, a seed at a re-mint that
is a restart (F-347). `readings` went to 706: twenty of round 89, in
`_round89_readings` (round 88's thirteen went to 686).
Core 0.64, the three profiles #935's fifth part rewrote, and round 87's
answers changed no fixture, and no rule but `desired.v1` 0.10's §2.16, a
`config.v1` owner's re-mint a restart (F-336). `readings` went to 686:
thirteen of round 88, in `_round88_readings` (round 87's nine went to 673).
Core 0.62, `observer.md` 0.6 and the five profiles' texts changed no
fixture, and no rule but §5.2's watch after a window, which `alarms.v1`
§2.15 now cites (round 86). `readings` went to 664: seven of that
watch, in the text and in zk2py's alarms reader.
Core 0.61 and `observer.md` 0.5 changed no fixture, and no rule but
§1's gained wording (round 85). `readings` went to 657: twelve of 0.5,
§1's NotAsked that spells not this profile's keeping its reason in every
medium over the 15 such answers of the three fixtures the rule cites, in
zk2py's freshness tool's member field and its typed boundary (review-bot
on PR #957), and §5.2's watch after a window (F-320).
Core 0.60 and `observer.md` 0.3 and 0.4 changed no fixture, and 0.60 and
0.4 no rule (round 84). `readings` went to 645: 36 of 0.3's two rules zk2py
now meets (five after review-bot on PR #955), §1's two spellings of not this profile's over the six fixtures
it cites (`zk2py.judgement`) and §8.2's NDJSON medium (`zk2py.echo_tool`).
Core 0.59 and `observer.md` 0.2 changed no fixture: their evidence is
three scenario steps, which `zk2py.observer_scenarios` runs (round 83).
`readings` went to 609: 22 of 0.2's rules where zk2py meets them (one
after review-bot on PR #952), the steps' two contracts, and four of
F-291's reading.
Core 0.57 added two contract fixtures (112): `ok-cardinality-unbounded`,
which failed its fingerprint and canonical bytes until zk2py resolved
`"unbounded"` to 4294967295 (round 82), and `e000-cardinality-string`,
which passed already. `readings` went to 581: 13 of `"unbounded"`, 48 of
what `observer.md` 0.1 asks of zk2py's tools (17, then 31 more after
review-bot on PR #949).
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
  fan-out (O2), with consolidation `None`. It returns a value, an
  owner's error, a transport error, or silence (O5). Since core 0.94
  (round 101) it writes the request form and reads the value and error
  forms. Since core 0.99 (round 104) it reads them alone: a reply under
  another `Encoding` is malformed, and a reply error under one the
  transport's.
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
- **Presence completeness (observer §3.3, §3.4).** A liveliness GET is complete when the
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
  - `live.check_s4` reads observer §4.2's two selectors, `@/*/router` and, since
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
ending `…/storage_manager/storages/<name>` (observer §4.2, 0.15). Under 0.11's
selector a router's `router/queryable/<…/state/**>` records answered it;
under 0.48's none does (see "A router's admin space beside a router that
is declaring").

**Since 0.16 and 0.17** (run `0.16`, and the `acl` run under `deny`):
- **A tool's S1 check (observer §4.4)** is `live.s1_check`. It judges the stamp
  first, then compares `meta.zid` with the zids it knows to be routers,
  by value. Its poles:
  - a foreign stamp is a finding, whether or not a router is verified;
  - an owner in client mode under a verified router is clean;
  - an owner opened in router mode and linked to R1 (`Owner(router_connect=…)`)
    is unobservable;
  - with no router's own answer counted, an owner's own stamp is
    unobservable (0.18).
- **O3 judged from outside (observer §7.2)** is `live.o3_verdict`. It takes the
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
  - `read_service` is a tool's verdict per resource, as scenarios §6 reads,
    each member's verdict with its reason (`member_fields`, observer §1,
    text 0.5: a deleted member is not asked, `deleted`, never a bare not
    asked). Its callers read the members through `service_members`, which
    refuses any other shape (`member_verdicts`).
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

## The population budget (core §2.7 and observer §5, 0.24, 0.25 and 0.27)

- **The `budget` conformance family:** `bounds.json`, `population.json`,
  `rate.json` and `lossless.json`, 95 of 95.
- **Lost no delivery is established** (0.25). `budget.lossless` takes the
  resource's declared QoS, the reader's own loss and miss detection. Only a
  `reliable`, `block` resource's window can be lossless. A `Window` counts
  its span from its declaration plus the reply timeout, and watches the
  owner's token until one lease after its end, plus the reply timeout:
  the longer of the lease and `wait_before_close` (`watch_after_s`, observer
  §5.2, text 0.5; F-320's reading A), a caller's own lease read the same
  way.
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

## A late joiner's seed (core §4.2 and observer §4.1, 0.26 and 0.27)

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
  - **After the seed** (core §4.3, 0.67, F-368): once its GET has ended, a
    consumer's late joiner orders each live sample by S7's consumer rule
    (`seed.follows`): by time within one id, and the first value under an
    id other than the held one's kept, though its time be lower, its
    ordering restarted there. A tool's keeps §4.2's merge, since S7 binds a
    consumer. Each arrival records the rule that decided it (`merge` or
    `S7`). The switch follows the order in which the callbacks were
    handed their arrivals, through one queue drained under one lock, so a
    sample that arrived before the GET's final reply is merged by §4.2.
    Round 90's F-382 and F-383 say what the text leaves open; under F-383's
    reading (a), two live writers under two ids make a key flap.
- **The owner** lets a test hold a state GET: `state_hold` takes it,
  `state_answer` makes the answer, and `answer_state` sends it.
- **`just py-state`** (`python -m zk2py.state_scenarios`) runs state.md §10
  against zk2py's owner, plus the timeout and absent-owner cases, and §11
  (0.27), 15 of 15. Since round 73 it runs §3 and §12 too (core 0.48,
  below), 22 of 22 in all. Since round 90 it runs §7 step 2 (S7, core
  0.67): a consumer's late joiner seeds rev 10, the owner restarts as a new
  instance on a new session with its clock 5 s behind (`Owner.clock`) and
  writes rev 11, and the consumer keeps rev 11, then rev 12 under the new
  id; a tool's late joiner, the control, keeps rev 10. 5 of 5. §11's W is a peer session without an HLC, which puts
  unstamped samples and answers a held GET with stamps zk2py chooses. The
  Rust owner example does not apply to §11: its keys are an owner's,
  always stamped.
- **S7 across the two implementations** (the live runner's `--only s7`, 6
  checks, round 90): the owner example restarted 5 s behind, and zk2py's
  consumer keeping its new epoch's value after its seed.
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
    reply timeout, ended before the instant judged by the watch observer
    §5.2 states, the longer of one lease and `wait_before_close`, plus the
    reply timeout (`window_end_ns`, round 86; until then one lease and the
    reply timeout, as §2.15 said before observer 0.6), lossless by the QoS
    and the reader's own receive path. A loss
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

## A router's admin space beside a router that is declaring (observer §4.3 and core Appendix B, 0.48)

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

## Core 0.57 and 0.58, and `observer.md` 0.1 (round 82)

**Core 0.57** names the no-ceiling value: `cardinality = "unbounded"`
(authoring format draft 2), "4294967295 from resolution on" (§9.1).
`zk2py.contract` resolves it so (`resolve_cardinality`), so a contract
writing it has the number's canonical form and fingerprint, and the bound
(§2.7), D007 (§3.3) and the compatibility check (§9.8) read the two
spellings alike. Any other string is E000, by `contract.schema.json`'s
`Cardinality`. 0.57's "At scale" lines are informative.

**Core 0.58** moved the reader's discipline to `spec/observer.md`, a
normative text of its own (text 0.1), word for word, a pointer left at
each old home. zk2py implemented those rules from the core in rounds 1
to 81, and its modules keep them:

| zk2py | Now |
|---|---|
| `live.list_presence`, `settle`, `Flight`, `Barrier`; `presence` | observer §3 (core §8.1's "Reading presence"; §3.10 is §8.3's count) |
| `live.presence_shapes`, `presence_faults` | observer §3.2 |
| `seed.LateJoiner` as a tool's seed | observer §4.1 (the order and the merge stay core §4.2's) |
| `live.check_s4`, `verified_routers`; `live.admin_guard`, `admin_records` | observer §4.2, §4.3 |
| `live.attribute_stamp`, `s1_check`, a zid by value | observer §4.4 |
| `budget.population`, `rate`, `lossless`, `StallWatch`, `Window` | observer §5 (the bound, a member, the owner's rule and the rate stay core §2.7's) |
| `redundancy_live.split_findings` | observer §6.1, §6.2 |
| `live.o3_verdict` | observer §7.2 |

**Citations.** Code that cites a moved rule writes `observer §n`, and a
version in parentheses stays the core amendment that made the rule, as
observer §0 says; a bare § in a core module is still the core's.
`SPEC-FINDINGS.md`'s headers of findings against a moved rule end "(now
observer §n)". Their quotes are history, as quoted.

**What the move lost.** No rule zk2py implements. Every sentence 0.58
removed stands in `observer.md` or the core, and every citation but one
moved with it (F-284). See "At core 0.58 and observer.md 0.1" in
`SPEC-FINDINGS.md`: F-281 to F-290, ten, none a blocker, none changing a
verdict.

**What `observer.md` asks that zk2py did not do**, now done:
- §8.2: `live.blind_spots(selector)`, the verbatim kinds a selector never
  reaches, read with zenoh's own intersection against each kind's key
  forms as core §1.1 and §2.5 write them (`live.VERBATIM_REACH`, each
  pattern cited beside it). `timing_tool`'s report
  carries `scope`: the kinds excluded and the timed streams of the
  revisions held that its selector misses, said on stderr too.
- §8.3: `timing_tool`'s `clocks`, whose clock its times and a lifespan's
  age are on.
- §7.4: `live.memlock_note()`, the largest SHM pool the soft
  `RLIMIT_MEMLOCK` admits, with core §7.4's words, said at start by
  `timing_tool` and `arbitration_tool` through `live.memlock_said()`, so
  that any failure reads "the memlock limit could not be read: …" and the
  tool goes on (F-290).

**Live, round 82.** `presence_scenarios` 95 of 95 (zk2py's owner and
reader), 49 (the Rust owner, §9 and §10), 9 and 8 (zenctl, §10);
`live_interop` 459 of 459; `state_scenarios` 32 of 32 with each reader;
`budget_scenarios` 41 of 41 with each owner; `timing_interop --only 6
--only face` 68 of 68. Beside §6, both timing tools over a plain and an
explicit timed stream under `zk2/lab/beat/**` watched the plain one alone;
zk2py's named the other unreached, zenctl's said nothing (F-287).

## `observer.md` 0.2 and core 0.59 (round 83)

**Core 0.59** names `observer.md` in its preamble and in §12's levels
(F-285), and adds three scenario steps for three of its rules.
**`observer.md` 0.2** answers round 82's other findings: evidence cited and
indexed (F-281 to F-284, F-287), §1's "not this profile's" (F-286), and
wording (F-288 to F-290). No rule changed and no fixture moved.

What zk2py does for 0.2:
- **§3.2's transient half** (operations.md §8 step 6).
  `live.presence_faults(…, reads=3, log=…)` reads a grace apart as many
  times as asked, and `live.shape_faults` pairs each read with the next:
  a shape in one read alone passes. `Owner(interface_tokens_after_s=1.0)`
  declares its interface tokens that long after its instance token, and
  `None` never does, the step's control.
- **§7.2 in the step's words** (operations.md §5 step 2).
  `live.o3_verdict`'s finding says beside it that an access-control
  refusal is silent too (`TOLD_FINDING`). Not told, it names the premise
  it lacked: grants that let it call, which no tool can observe
  (`NOT_TOLD`).
- **§7.4's report.** A limit the tool cannot read reads "the memlock
  check is unobservable: the limit could not be read: …". `timing_tool`
  and `arbitration_tool` carry the note in their JSON (`memlock`) as well
  as on stderr.
- **§8.2's scope** (grammar.md §1 step 5). `zk2py.echo_tool` is a generic
  watcher: its report's `scope` holds the kinds it `watched` and the
  verbatim kinds it `excluded`, with the sentence §8.2 asks for. The same
  sentence goes on stderr. `timing_tool` says what its selector excludes
  in its `notes` and on stderr, whenever it excludes anything.
- **§1's not this profile's**: nothing to change. zk2py keeps the token
  where `timing.v1`, `arbitration.v1` and `redundancy.v1` give it.
  `freshness.v1`, `telemetry.v1` and `desired.v1` give it as not asked, and
  zk2py follows their fixtures (F-291).

`python -m zk2py.observer_scenarios [--only grammar1|ops5|ops8]
[--owner zk2py|rust]` runs the three steps on in-process routers:
- **`grammar1`**: grammar.md §1 steps 1 to 4 with zk2py's consumer, and
  step 5 with four tools: zk2py's echo tool, `zenctl echo` (table and
  ndjson), `zenctl timing` over `zk2/g/**`, and zk2py's timing tool over
  `zk2/g/svc/**` (it reads one owner). The owner of `zk2/g/svc`
  (`interop/grammar/zk2py_guard.v1.toml`: every kind token, both streams
  timed) is zk2py's or the reference's, driven by its `put` command.
- **`ops5`**: operations.md §5 steps 1 and 2, under zk2py's generated
  `deny` grants on a usrpwd router. One principal is granted the call
  (`interop/operations/zk2py_frozen.v1.toml`'s one operation), and another
  is granted presence alone. Both zk2py's tool and `zenctl check conform
  --i-know`, with and without `--calls-granted`, are judged.
- **`ops8`**: operations.md §8 step 6 with zk2py's owner and two tools:
  zk2py's (three reads 1.5 s apart) and `zenctl doctor --check
  token-missing`. Each runs beside the control. The reference's owner
  cannot pause between its tokens.

**Live, round 83.**
- `observer_scenarios`: 38 passed, and 2 XFAIL by F-292 (`zenctl echo
  --format ndjson` says nothing of its scope).
- `presence_scenarios --only 9 --only 10`, by owner and reader:

  | Owner, reader | §9 | §10 |
  |---|---|---|
  | zk2py, zk2py | 9 | 46 |
  | Rust, zk2py | 9 | 40 |
  | zk2py, zenctl | – | 9 |
  | Rust, zenctl | – | 8 |

- `live_interop --only tool-rules --only acl --only 0.16`: 53 of 53.

Each pairing's results are in `SPEC-FINDINGS.md`, "At observer.md 0.2 and
core 0.59". Round 83 found F-291 to F-294.

## `observer.md` 0.4 and core 0.60 (round 84)

**`observer.md` 0.3** answers round 83's F-291 to F-294: §1 names both
spellings of "not this profile's", §8.2 names the medium a scope's
exclusions go in, §3.2's transient half is within core §8.2's step 4, and
§5.3 cites `budget.md` §1–§3. **`observer.md` 0.4 and core 0.60** take the
inline history out of the rules (#935): version and finding tags,
measurements (now in core Appendix B, each rule keeping a pointer and its
reason in words) and notes about one implementation, zk2py's among them.
Both changelogs claim no rule changed. zk2py checked the claim: every
RFC 2119 sentence before and after, by section, and the word diff of both
texts whole. No keyword, condition or scope moved, every measurement is
reachable from its rule, and no reason was lost. See "At observer.md 0.4
and core 0.60" in `SPEC-FINDINGS.md`.

What zk2py does for 0.3:
- **§1's two spellings** (`zk2py.judgement`). `read(profile, answer,
  reasons)` reads a profile's answer into §1's shape: Established,
  NotAsked, Unobservable, or not this profile's. Not this profile's is
  the token `not_this_profiles`, or a `not_asked` whose reason §1 names
  for that profile (`SPELLINGS`). `cell`, `field` and `note` render the
  three media. Not this profile's is never Unobservable nor a negative
  pole in any of them, and the profile's own token stays in the field. A
  NotAsked of the second spelling keeps its reason in the cell, which §1
  does not ask (F-295).
- **§8.2's medium** (`zk2py.echo_tool --format ndjson`). One JSON line per
  sample, the row alone. The scope goes to standard error in every format
  (`scope_line`), before the first row. The JSON document keeps its
  `scope` field. zenoh's callback only queues each row (`Rows`), and the
  main thread writes it. A write that fails is the stream cut: "echo: the
  stream was cut: …" on stderr, exit 2.

**Live, round 84.**
- `observer_scenarios`, whole: 46 passed, with no XFAIL; 48 after
  review-bot on PR #955, the cut stream checked with each owner.
  `zenctl echo --format ndjson` states its scope on stderr, so F-292's
  two `KNOWN` entries are gone, and zk2py's echo tool is checked in
  NDJSON beside its JSON.
- `presence_scenarios --only 9 --only 10`, by owner and reader:

  | Owner, reader | §9 | §10 |
  |---|---|---|
  | zk2py, zk2py | 9 | 46 |
  | Rust, zk2py | 9 | 40 |
  | zk2py, zenctl | – | 9 |
  | Rust, zenctl | – | 8 |

- `live_interop --only tool-rules --only acl --only 0.16`: 53 of 53.

Round 84 found F-295 to F-314. F-295 is a gap (what a NotAsked of §1's
second spelling shows in a cell), and F-296 is minor (two reasons in words
wider than their measurement). F-297 to F-314 are the new class, **hard to
read**: eighteen sentences zk2py read twice, each with a rewrite that
keeps its rule, but F-298, which is ambiguous.

## `observer.md` 0.5 and core 0.61 (round 85)

**`observer.md` 0.5 and core 0.61** answer round 84's F-295 to F-314
(#935's second part). They split every sentence over 50 words in both
texts. §1 gains one wording, F-295's: a NotAsked that spells not this
profile's "keeps its reason in every medium … never show a bare not
asked". §3.3 restores 0.3's meaning for F-298: the SHOULD measures the
reader's own wait, never its GETs' timeout. Both changelogs claim no
other rule changed. zk2py checked the claim with every RFC 2119 sentence
before and after, by section, and the word diff of both texts whole.
- **Observer:** 39 before, 43 after, 31 unchanged.
- **Core:** 3 changed, each keeping its keyword, condition and scope.

No rule zk2py implements changed. See "At observer.md 0.5 and core 0.61"
in `SPEC-FINDINGS.md`.

What zk2py does for 0.5:
- **§1's F-295 in its tools.** `judgement.cell` kept the reason since
  round 84. `freshness.read_service`'s member field gave a verdict token
  alone, so a deleted member read a bare `not_asked`. Each member is now
  its verdict and its reason (`freshness.member_fields`). Its callers
  read it through `freshness.service_members`, which refuses any other
  shape, a bare token among them (review-bot on PR #957).
- **§5.2's watch after a window** (F-320). `budget.watch_after_s` is the
  longer of the lease and `wait_before_close`, plus the reply timeout.
  Before, zk2py's watch was the lease alone, whatever `wait_before_close`
  was. A caller's own lease is read the same way, so no watch is shorter
  than `wait_before_close` plus the reply timeout. Under zenoh's defaults
  nothing changes. `budget.md` §1's tool
  window, with a 1 s lease, now watches 6 s after its end.

**Live, round 85.**
- `observer_scenarios`, whole: 48 of 48, with no XFAIL.
- `presence_scenarios --only 9 --only 10`, by owner and reader:

  | Owner, reader | §9 | §10 |
  |---|---|---|
  | zk2py, zk2py | 9 | 46 |
  | Rust, zk2py | 9 | 40 |
  | zk2py, zenctl | – | 9 |
  | Rust, zenctl | – | 8 |

- `freshness_scenarios`: 21 of 21, then 23 of 23 with §6's member
  checks. `budget_scenarios --only 1b`: 2 of 2.
- `live_interop --only freshness --only budget --only tool-rules`: 46 of
  46 twice. One run read 45: `lab/rw`'s owner was absent at one window's
  reading, on a path round 85 did not change, and the failure did not
  come back. After review-bot on PR #957, `--only freshness --only
  budget` passed 32 of 32. Two other runs failed the freshness run's
  first descriptor GET, right after the owner example was ready, which
  comes before anything round 85 changed. It passed in four of six runs.

Round 85 found F-315 to F-323, nine, all of the class **hard to read**:
- eight are hard to read, each with a rewrite that keeps its rule;
- **F-320** is ambiguous: what a `wait_before_close` longer than the
  lease does to the watch after a window.

None is a blocker. Only F-320 could move a verdict, and only under that
configuration.

## `observer.md` 0.6, core 0.62 and five profiles without history (round 86)

**`observer.md` 0.6 and core 0.62** answer round 85's F-315 to F-323
(#935's third part). Every one is resolved. F-320 is settled as zk2py's
reading A: after a window's end, a tool watches the owner's instance
tokens for the longer of one lease and the routers' `wait_before_close`,
plus its reply timeout. **`freshness.v1` 0.5, `health.v1` 0.5,
`hostid.v1` 0.8, `config.v1` 0.6 and `link.v1` 0.9** take their inline
history out of the rules and split their sentences over 50 words. Each
changelog claims no rule changed. zk2py checked the claim with every RFC
2119 sentence before and after, by section, and read each profile's word
diff whole, in full context, as its cold read:
- **No rule zk2py implements moved** in the five profiles. Each keyword
  set is the changelog's. Every rewrite keeps its condition, object and
  scope.
- **`alarms.v1` §2.15 did change**, under its text version 0.8 (F-326):
  it now watches the owner past a window as observer §5.2 says.

See "At observer.md 0.6 and core 0.62, and five profiles without
history" in `SPEC-FINDINGS.md`.

What zk2py does for 0.6:
- **§5.2's watch, in the alarms reader.** `budget.watch_after_s` has had
  reading A since round 85. `AlarmsReader` still ended its window one
  lease and the reply timeout before the instant it judged, as §2.15
  said. It now ends it by `alarms.window_end_ns`, the budget's length, and
  `alarms_scenarios` §10 listens for it, its wait bounded (review-bot on
  PR #959). Under zenoh's defaults nothing changes. `AlarmsReader.offline`
  builds a reader with no session, through its constructor, for readings
  of `window()`.
- **The five profile modules** say in their headers that they were read
  again against the new texts.

**Live, round 86.** No check failed, and no run was repeated.
- The five profiles' runners: `freshness_scenarios` 23 of 23,
  `health_scenarios` 29 of 29, `hostid_scenarios` 35 of 35,
  `config_scenarios` 51 of 51 with zk2py's owner and 51 of 51 with the
  Rust owner example, `link_scenarios` 89 of 89 (zk2py's generator alone).
- `live_interop --only freshness --only health --only hostid --only
  config --only budget`, against the owner and consume examples and
  zenctl: 182 of 182.
- `observer_scenarios`, whole: 48 of 48.
- `presence_scenarios --only 9 --only 10`, by owner and reader:

  | Owner, reader | §9 | §10 |
  |---|---|---|
  | zk2py, zk2py | 9 | 46 |
  | Rust, zk2py | 9 | 40 |
  | zk2py, zenctl | – | 9 |
  | Rust, zenctl | – | 8 |

- `alarms_scenarios --only 10`, the alarms reader's regression: 17 of 17
  with zk2py's owners, and 17 of 17 with the owner example's.

Round 86 found F-324 to F-335, twelve:
- **ten hard to read**, F-325 and F-327 to F-335, each with a rewrite
  that keeps its rule. Six are in sentences #935's third pass wrote or
  split (F-325, F-328, F-331, F-332, F-334, F-335);
- **F-324**, a gap: §5.2's reason for the longer watch says a router
  tells the withdrawal `wait_before_close` late, and the scenario it
  cites, run beside a `wait_before_close` longer than the lease, shows it
  told one lease late. No measurement shows the half 0.6 settled;
- **F-326**, minor: `alarms.v1` 0.8's §2.15 changed under one version,
  so a reader of "0.8" cannot tell which watch it holds.

None is a blocker, and none moves a verdict.

## `alarms.v1` 0.9, `telemetry.v1` 0.6, `desired.v1` 0.9, `observer.md` 0.7 and core 0.63 (round 87)

**#935's fourth part** takes the inline history out of `alarms.v1`,
`telemetry.v1` and `desired.v1`, and splits their long sentences. Each
changelog claims no rule changed. The one exception is `alarms.v1` 0.9's
§2.15, which now holds the watch after a window under a version of its own
(F-326).
**`observer.md` 0.7, core 0.63, `health.v1` 0.6, `hostid.v1` 0.9,
`config.v1` 0.7 and `link.v1` 0.10** answer round 86's F-324 to F-335.
Every one is resolved. F-324's answer is a statement, not a scenario step:
the `wait_before_close` half of §5.2's watch is a bound reasoned and not
measured, and core Appendix B says so in a "Not measured" line.

zk2py checked the claim with every RFC 2119 sentence before and after, by
section, and read each profile's word diff whole, in full context, as its
cold read. **No rule zk2py implements moved.** `desired.v1`'s 60 keyword
sentences are 62, two that each held two MUSTs split, and each changed
sentence keeps its keyword, condition and scope. See "At alarms.v1 0.9,
telemetry.v1 0.6, desired.v1 0.9, observer.md 0.7 and core 0.63" in
`SPEC-FINDINGS.md`.

What zk2py does for them:
- **`readings`: 673, up from 664.** `_observer_07_readings` pins the
  answers to F-324, F-325 and F-326 as the texts state them, and reads
  three of `desired.v1` 0.9's split rules through `desired.TargetRole`:
  §2.6 step 4's three sentences, step 3's "Only a version from an archive
  is confirmed" over five deliveries, and §2.7's restart.
- **The modules' headers** (`alarms`, `telemetry`, `desired`,
  `desired_live`, `desired_tool`, `config`, `health`, `hostid`, `link`)
  say they were read again against the new texts. `budget.watch_after_s`
  quotes observer 0.7's words.
- **Nothing else.** The texts ask nothing new of zk2py.
- **`just py-conformance`: 3630 of 3630.**

**Live, round 87.** No check failed, and no run was repeated.
- `alarms_scenarios`: 80 of 80 with zk2py's owner, and 80 of 80 with the
  owner example (§2, §4, §6 and §10 on the 4 s copy).
- `telemetry_scenarios`: 57 of 57 with zk2py's owner and 56 of 56 with
  the owner example; with `zenctl telemetry` judging beside zk2py's
  reader, 164 of 164 and 163 of 163.
- `desired_scenarios`, `zenctl desired` as §9's Rust tool:

  | Author → target | Watch | Polling |
  |---|---|---|
  | zk2py → zk2py | 175 of 175 | 175 of 175 |
  | Rust → Rust | 85 of 85 | 85 of 85 |
  | zk2py → Rust | 85 of 85 | 85 of 85 |
  | Rust → zk2py | 85 of 85 | 85 of 85 |

  Each run counts §1 to §8, §10 and §11 (45), the edges (9), the re-mint
  (2) and §9 (29); zk2py → zk2py adds the thirty shapes (90), each asked
  of both tools.
- The regression, `config_scenarios`: 51 of 51 with zk2py's owner, and
  51 of 51 with the owner example.
- **Not run:** `alarms_scenarios` §6 and §10 at 900 s, and the
  `live_interop`, observer and presence runners, whose texts changed only
  in wording.

Round 87 found F-336 to F-346, eleven:
- **F-336**, a contradiction that predates #935. A `config.v1` owner fed
  through `desired.v1` that re-mints is a restart by `config.v1` §2.12,
  acting again, and keeps its run by `desired.v1` §2.7, acting on nothing.
  zk2py's config owner is never fed through `desired.v1`, and the
  reference refuses the re-mint, so no run meets it;
- **ten hard to read**, F-337 to F-346, each with a rewrite that keeps
  its rule. Five are in sentences #935's fourth pass wrote or split
  (F-337, F-342, F-344, F-345, F-346).

None is a blocker, and none moves a verdict of zk2py's.

## `redundancy.v1` 0.6, `timing.v1` 0.6, `arbitration.v1` 0.4, `desired.v1` 0.10 and core 0.64 (round 88)

**#935's fifth part** takes the inline history out of `redundancy.v1`,
`timing.v1` and `arbitration.v1`, of the core's scenarios (core 0.64) and
of the profiles' index, and splits their long sentences. Each changelog
claims no rule changed. **`alarms.v1` 0.10, `telemetry.v1` 0.7,
`desired.v1` 0.10 and `config.v1` 0.8** answer round 87's F-336 to F-346.
Every one is resolved. F-336's answer is the one rule the pass changed:
for a `config.v1` owner fed through `desired.v1`, a re-mint is a restart,
a new run that acts on the bound version again (`desired.v1` §0, §2.7,
§2.16, its scenarios' new §12; `config.v1` §2.18).

zk2py checked the claim with every RFC 2119 sentence before and after, by
section, compared every Expected item of the scenarios with its tags taken
out, and read each word diff whole, in full context, as its cold read.
**No rule zk2py implements moved** but F-336's. See "At redundancy.v1 0.6,
timing.v1 0.6, arbitration.v1 0.4, desired.v1 0.10 and core 0.64" in
`SPEC-FINDINGS.md`.

What zk2py does for them:
- **`desired.v1` §2.16, implemented.** Until now zk2py's `config.v1`
  owner read a desired document bound to it directly, never through
  `desired.v1`. `ConfigOwner(fed=(slice, author))` now reads its slice as
  `desired.v1` §2.5's late joiner does (subscribe, then seed, again at each
  appearance of its author's instance token, read every 500 ms on its own
  session), judges every version with `desired.TargetRole`, acts through
  `Core.desire`, and on a delete reverts what the documents it acted on in
  this run named (`Core.revert`: one `set` per group, the refused documents
  counting for none, no `desired` after it). `ConfigOwner.remint()`
  re-mints as a restart: its device returned to its startup values first,
  the new instance's view and status put before its tokens, make before
  break, then a new run that seeds and acts again (F-347's reading). With
  `remint="refuse"` it refuses, as the reference does.
- **`desired_scenarios` §12**, the runtime's, with zk2py's own parties in
  process: `lab/author` a zk2py owner of `interop/desired/configs.v1`, and
  `lab/rf0` zk2py's config owner, its role `configs` declared by
  `interop/desired/zk2py_configured.v1`. Step 1 runs both branches, an
  owner that refuses and one that re-mints as a restart; step 2 restarts
  the process. `config-fed` is zk2py's own: an equal document adopted, a
  reach document refused whole, a delete's revert. The owner example has
  no such owner, so both are NOT RUN beside it.
- **`redundancy_scenarios` §7** reads B's marked descriptor since the
  heal, never against its token's withdrawal: "S's arrival order of a put
  and a token is noted, never judged" (the conventions). A Rust B's marked
  put once reached S first, and the check failed on the order alone.
- **`readings`: 686, up from 673** (`_round88_readings`, thirteen).
- **The modules' headers** (`config`, `desired`, `desired_scenarios`,
  `redundancy`, `timing`, `arbitration`) say they were read again.
- **After review-bot on PR #963** the feed never ends silently: a read
  or a seed that raises is counted (`fed_errors`, `fed_last_error`) and
  the loop goes on, the re-mint's seed is made again as `_fed_poll`'s is,
  an act that raises is `failed`, code `apply`, and an act's end reaches
  the role even when a put raises. A refused document leaves the role free
  for the next version: it always did, since `Core.desire` sets
  `desired_end` before `_after` hands it on; a reading and a `config-fed`
  check now pin it.
- **`just py-conformance`: 3643 of 3643.**

**Live, round 88.** Every check passed but one of a runner's own, fixed and
run again (below). "Rust" is the owner example, "zenctl" the reference tool.
- `redundancy_scenarios`, by (A, B): zk2py, zk2py **105 of 105**; zk2py,
  Rust **104 of 104**; Rust, zk2py **103 of 103**; Rust, Rust **101 of 102**,
  the failure §7's order check above, then `--only 7` 7 of 7 in each of
  three runs. §12 33 in each, both tools.
- `timing_interop`, every owner:reader pairing: **182 of 182**, §6 and its
  extras with zk2py's tool and `zenctl timing` beside each owner.
- `arbitration_scenarios`, every commanders:actuators pairing: **329 of
  329**, §10 with both tools.
- `presence_scenarios`, by (owner, reader): zk2py, zk2py **95 of 95**;
  Rust, zk2py **89 of 89**; zk2py, zenctl **33 of 33**; Rust, zenctl **32
  of 32**. A step a tool's command line cannot show is NOT RUN, as before.
- `desired_scenarios`, `zenctl desired` as §9's Rust tool:

  | Author → target | Watch | Polling |
  |---|---|---|
  | zk2py → zk2py | 188 of 188 | 188 of 188 |
  | Rust → Rust | 85 of 85 | 85 of 85 |
  | zk2py → Rust | 85 of 85 | 85 of 85 |
  | Rust → zk2py | 85 of 85 | 85 of 85 |

  zk2py → zk2py adds §12 (9) and `config-fed` (4) to round 87's 175. Beside
  the owner example both are NOT RUN: it has no such owner.
- The regression, `config_scenarios`: **51 of 51** with zk2py's owner, and
  **51 of 51** with the owner example.

Round 88 found F-347 to F-357, eleven:
- **F-347**, a gap: a `config.v1` owner's re-mint begins a new run, and
  nothing says the run reads its slice (`desired.v1` §2.5 seeds at a start
  and at its author's appearance). Without a seed, §12's re-minted owner
  would never act again. zk2py's seeds;
- **ten hard to read**, F-348 to F-357, each with a rewrite that keeps its
  rule. Six are in sentences #935's fifth pass wrote or split.

None is a blocker, and none moves a verdict.

## Core 0.66, `observer.md` 0.8 and `spec/RULES.md`: the classes zk2py claims (round 89)

**Core 0.66 claims conformance by role** (§12, "Conformance classes"). Each
of §0's owner, consumer, caller, deployment and tool roles has a class: its
rules in `core.md` and `observer.md`, and the static rules. `spec/RULES.md`,
generated from both texts, lists every MUST, MUST NOT, SHOULD and SHOULD NOT
by the role it binds. `observer.md` 0.8's §0 says where its rules belong,
and `spec/README.md` opens with a glossary of 36 terms. #935's sixth part
answers round 88's F-347 to F-357. Every one is resolved, and F-347's is
the one rule it changed: `desired.v1` 0.11 §2.5 seeds at a re-mint that is
a restart, which zk2py's owner did already.

zk2py checked the claim that no rule changed. It diffed every RFC 2119
sentence of the two texts before and after, by section, and read each of
the nineteen core and three observer sentences that now name their role
against its old text. **No rule moved.** See "At core 0.66, observer.md
0.8 and spec/RULES.md" in `SPEC-FINDINGS.md`.

### The classes zk2py claims

| Class | Claimed | Level | Where |
|---|---|---|---|
| owner | yes | full | `owner.Owner`, and each profile's owner built on it |
| consumer | yes | full | `Owner.subscribe_role`, `live.get_state`, `seed.LateJoiner`, the profiles' consumers |
| caller | yes | full | `live.call`, `live.retrieve_bundle` |
| observer (the tool role) | yes | full | `echo_tool`, `timing_tool`, `arbitration_tool`, `desired_tool`, the readers of `live` and `presence`, and the grant generator `acl` |
| deployment | **no** | | zk2py configures no deployment. Its runners bring up routers for a scenario, and its generators (`acl`, `link`) are tools (core §0, §11.1). |

"Full" is read as the level without the constrained modifier (F-358 asks
which §12 means). The static rules are in every class.

**The profiles' classes**, claimed by following each profile (§12), by the
roles zk2py plays in it:

| Profile | Roles zk2py plays |
|---|---|
| `hostid.v1` | owner (a minted system), tool (the duplicate check) |
| `freshness.v1` | owner (re-puts), consumer and tool (the judgements) |
| `health.v1` | owner, tool (§2.11's reader and the roll-ups) |
| `config.v1` | owner (the simulated device), caller, tool (§5's reader) |
| `link.v1` | tool (the generator and a block's checks) |
| `alarms.v1` | owner, tool (the reader and §5) |
| `telemetry.v1` | owner, tool (§5's reader) |
| `desired.v1` | owner (an author), consumer (a target), tool (§2.17); no archive's last-known |
| `redundancy.v1` | owner (a candidate), consumer (the follower), tool (§5) |
| `timing.v1` | owner (the stamping clock), consumer (deadlines, lifespans), tool (§2.7, §5) |
| `arbitration.v1` | owner (a commander), consumer (an actuator), tool (§2.14, §5) |

### Its walk of `RULES.md`

`zk2py/classes.py` gives a verdict to every `RULES.md` row of the four
classes and of the static rules, 158 rows at f07f2287, each with its status
and the module or fixture family that shows it.
- **Keyed to the rule, not to the index.** A verdict names its rule by its
  section and a phrase of its own sentence, as `core.md` or `observer.md`
  states it (`classes.norm`). It walks the row of its class and section
  that shows that phrase, whatever antecedent in brackets, "(n items)" or
  evidence the row shows around it. So a regenerated `RULES.md` moves no
  verdict.
- **A row with no verdict is reported, never failed.** `just
  py-conformance` prints it below the totals, `NOTE readings: classes: no
  verdict for <class> <section> <rule>: '<sentence>'`, with the counts, and
  the next round gives it its verdict. At f07f2287 there is none.
- **The one failure** is a verdict whose rule its text no longer states,
  named (`classes.check`).
- `python -m zk2py.classes` prints the walk as a table, and the same report
  on standard error.

The `RULES.md` rows at f07f2287, by class:

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 56 | 45 | 3 | 8 |
| consumer | 12 | 7 | 2 | 3 |
| caller | 11 | 11 | 0 | 0 |
| observer | 46 | 44 | 2 | 0 |
| static, in every class | 33 | 31 | 0 | 2 |

**Not met**, each a gap of zk2py's:
- **owner, three SHOULDs:** §1.4's canonical form before a slug (zk2py's
  owner slugs the value it is handed); §1.5's new session at a re-mint
  (`new_epoch` keeps its session, for `telemetry.v1` scenarios §2's "the
  process unchanged"); §3.3's descriptor within 1 KB (unchecked);
- **consumer, S7's two MUSTs:** `seed.LateJoiner` merges by core §4.2's
  order, time then id, which disagrees with S7 after a new epoch on a clock
  behind (F-368);
- **observer, §7.3's two decoding MUSTs:** the echo tool shows a sample's
  size and encoding, never its value or its declared type, and types.md §2
  is not run (F-370).

**Not applicable:** the five rules of an archive (zk2py runs none), the two
of the constrained level, §7.4's generated SHM publisher, S5 and S6 (no
last-known read), §11.1's advanced subscriber (zk2py's consumers declare
none), and §9.8's two publication rules (zk2py publishes no revision).

**Beyond the index.** Fourteen verdicts walk no row: rules the texts bind a
claimed class to and `RULES.md` does not list under it. They are §4.2's
late joiner for a tool, §8.4's holder, and §8.1's instance token for a
caller that is a service (F-361); `observer.md` §1's three, §2's, §5.2's,
§7.1's graph and §8's three, stated without a keyword (F-362); and §2.2's
"MAY appear only" (F-363). zk2py meets thirteen. The fourteenth, a bundle
holder's, binds no zk2py tool. When `RULES.md` lists one of them under its
class, its verdict walks that row.

### What the walk changed in zk2py

Five owner rules the walk found zk2py's owner not keeping, kept now:
- **§2.4, "An owner MUST publish with it":** `owner.qos_of` gives every
  publisher its resource's four fields. `reliability` is set through
  zenoh's unstable API (zenoh-python 1.10.1 marks `Reliability` unstable,
  and a publisher declared without it is reliable; F-369). An event, whose
  key is its occurrence's own (its ULID), went on `Session.put` with
  zenoh's defaults (congestion `drop`, not `block`). A reliable event, the
  default, now goes on `Session.put` with the three stable fields, zenoh's
  own reliability being reliable, and no publisher declared. A
  `best_effort` event alone goes on a publisher declared for its one put,
  since `Session.put` takes no reliability. An occurrence counts for its
  member's rate only once it is put: one that raises publishes nothing and
  consumes no rate.
- **§3.3, "The owner's descriptor MUST NOT raise it":** a stated
  cardinality above the contract's, below 1, of no templated resource, or
  not an integer (`"3"`, which a descriptor cannot carry), is refused at
  §8.2's step 2, nothing declared. Every caller in zk2py lowers a bound
  (`budget_scenarios`, `alarms_scenarios`, `live_interop`), and each still
  starts. No zk2py CLI takes a cardinality.
- **O6:** `OpCall.summary` refuses a second summary, a value after one, and
  a summary for an operation that declares none, each to the handler.
- **§5.2:** `OpCall.refuse` refuses a detail for an operation with no
  `error` type, nothing sent.
- **What a refusal to the handler sends.** A handler that does not catch
  one has failed, and `OpCall.finish` answers the call `internal`, never
  silence (O3, §5.1). The one exception is a refusal after the summary: the
  summary ended the replier (O6), so the call has its answer and nothing
  more is sent. No zk2py handler sends a summary, or a detail without an
  `error` type, so none relied on the old behaviour.
- **§5.1, "Beside replicas":** an instance whose exposed resources of an
  interface are all replicated operations declares no `unavailable`
  queryable on an exclusive one (`owner.replica_only`).

And observer §7.4: the echo tool says what its `RLIMIT_MEMLOCK` admits at
its start, as the timing and arbitration tools did. The readings use
`interop/classes/zk2py_replica.v1.toml`, three operations, lint-clean with
the reference CLI's fingerprint.

**`readings`: 706, up from 686.** **`just py-conformance`: 3663 of 3663.** Below the totals it
prints the walk's counts, its rows with no verdict, and `RULES.md`'s table
against its own entries, each a `NOTE`, never a failure.

**Live, round 89**, `presence_scenarios` against the black boxes, by
(owner, reader): zk2py, zk2py **95 of 95**; Rust, zk2py **89 of 89**; zk2py,
zenctl **33 of 33**; Rust, zenctl **32 of 32**. After the owner's changes,
zk2py, zk2py and zk2py, zenctl ran again: 95 of 95 and 33 of 33. A step a
tool's command line cannot show is NOT RUN, as before.

Round 89 found F-358 to F-378, twenty-one, none a blocker:
- **two ambiguous:** F-358, whether "full" is a level or every class, and
  F-370, which tool must decode;
- **two hard to read:** F-376, `observer.md` 0.8's "when it seeds", and
  F-377, §12's "as this text's do (its §0)";
- **the index:** roles left out (F-361), rules without a keyword (F-362), a
  RECOMMENDED and a "MAY … only" (F-363), a list's closing tag read as
  "none cited" and a wrong id (F-364), pronouns cut from their noun (F-366);
- **the glossary:** "Complete read" says less than observer §3, which
  moves a verdict (F-371), "Presence" says more (F-372), eleven more
  (F-373), "Ambient selector" against §1.3 (F-374), terms missing, "reader"
  first (F-375);
- **the texts:** where a static section's role rule goes (F-359), static
  rules that bind no role (F-360), thirteen MUSTs uncited (F-365), the
  changelog's counts (F-367), S7 against §4.2 (F-368, a verdict), a
  publisher's reliability unstable (F-369), and a stream-only owner's path
  through the whole of §9 (F-378).

## Core 0.67 and `observer.md` 0.9: round 89 answered (round 90)

Core 0.67 and `observer.md` 0.9 answer round 89's F-358 to F-378 (#926's
second part). Twenty are resolved; F-365 stays open as #965's. They change
four rules: §2.4's `reliability`, unstable API in zenoh 1.10.1, which an
owner sets through §0's opt-in or MAY publish reliable (F-369); S7, where a
late joiner's seed merges as §4.2 says and the consumer rule orders what
follows it (F-368); §1.3's guard, of any selector without a verbatim chunk
(F-374); and observer §7.3's decoding, of a tool that shows or judges a
payload (F-370). They restate two (§2.2's rest parameter as a MUST NOT,
§11.2's RECOMMENDED as a SHOULD), each the same rule. zk2py diffed every
RFC 2119 sentence of both texts: no other changed. But §0's and §12's "where
it does what they govern" narrows every static rule's reach (F-378's
reading (b)). It moves no zk2py verdict, since zk2py does every act the
static rules govern (F-392).

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level, which 0.67 now defines as a
class claimed without the constrained modifier (F-358). Not the deployment
class. The profiles' classes, by role, are round 89's table above.

### Its walk of `RULES.md` at d6eb62d2

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 56 | 45 | 3 | 8 |
| consumer | 13 | 10 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 50 | 46 | 2 | 2 |
| static, in every class | 34 | 32 | 0 | 2 |

- **166 rows walked, none without a verdict.** The two verdicts keyed to
  moved sentences are re-keyed (observer §7.3, core §2.2). The five new rows
  have theirs: §8.4's holder under consumer and caller (met), §8.1's
  instance token and §8.4's holder under observer (not applicable: no zk2py
  tool is a service or holds a bundle), and the late joiner's two MUSTs
  under observer (met), which round 89 gave beyond the index.
- **Beyond the index:** nine, the observer rules stated without a keyword
  (`observer.md` §1, §2, §5.2, §7.1's graph and §8), each met. Since 0.67 the
  texts say a class holds them and the index does not list them.
- **Not met:** the owner's three SHOULDs (§1.4's canonical form, §1.5's new
  session at a re-mint, §3.3's 1 KB), and observer §7.3's two MUSTs. 0.9's
  narrower subject does not make them not applicable: the echo tool shows
  each sample, its size and encoding in its payload's place, and the
  profiles' readers judge payloads with no protobuf decoder (F-379).
- **Met since this round:** S7's two consumer MUSTs.

### What round 90 changed in zk2py

- **S7 after the seed.** `seed.LateJoiner` merged by stamp for as long as it
  followed. Once its GET has ended, a consumer's now orders each live sample
  by S7's consumer rule (`seed.follows`): by time within one id, and a value
  under an id other than the held one's kept, its ordering restarted there.
  A tool's keeps §4.2's merge. Each arrival records the rule that decided it.
  The switch is decided by the order of arrival: each callback's first act
  queues what it was handed, and one lock drains the queue in order, so a
  sample that arrived before the GET's final reply is merged though its
  thread runs after the end's (review-bot on PR #969).
- **`state.md` §7 step 2, live,** by (owner, late joiners): zk2py, zk2py
  (`state_scenarios` §7) 5 of 5; Rust, zk2py (`live_interop --only s7`, the
  owner example restarted with `--clock-offset-ms -5000`) 6 of 6. The
  consumer keeps the new epoch's value, and the tool's late joiner, the
  control, the old.
- **F-369's opt-in** was set already (round 89), and so was F-368's merge
  by stamp inside the seed.

**`readings`: 717, up from 706.** **`just py-conformance`: 3674 of 3674.**

**Live, round 90**, `presence_scenarios` by (owner, reader): zk2py, zk2py
**95 of 95**; Rust, zk2py **89 of 89**; zk2py, zenctl **33 of 33**; Rust,
zenctl **32 of 32**, each as round 89's.

Round 90 found F-379 to F-392, fourteen, none a blocker:
- **three ambiguous:** F-379, which tool shows a payload and what it must
  decode; F-380, §2.4's "One that cannot set it" beside §0's stable API;
  F-383, S7's "a new id";
- **three hard to read:** F-389, §0's implementation role; F-390, §12's
  "these sections"; F-391, §2.4's "so none moves by it";
- **the texts:** §2.5's "the one place" an unstable API is opted into
  (F-381), §4.2's merge with no end (F-382, an implementation), §8.4's two
  MUSTs naming a caller alone (F-387), and the 0.67 entry's "moves no
  verdict" (F-392);
- **the index:** S2's "It [the owner's state queryables]" (F-384), two
  rules' evidence (F-385), §2.4's MUST shown without its MAY (F-386);
- **the glossary:** seven definitions that say less or more than their
  sections (F-388).

## Core 0.68 and `observer.md` 0.10: round 90 answered (round 91)

Core 0.68 and `observer.md` 0.10 answer round 90's F-379 to F-392 (#926's
third part). All fourteen are resolved, three with something left
(F-382's, F-386's and F-388's, carried to F-393, F-394 and F-399). They
change five rules: §2.4's `reliability`, which an owner that opts in sets
and one that does not MAY publish reliable (F-380); §4.2's merge and S7's
two consumer MUSTs, a consumer ordering by S7 from its first value, its
seed's replies included, and accepting a value under an id other than its
held value's, per key (F-382, F-383); §8.4's two MUSTs, which name "a
caller or tool" (F-387); and observer §7.3, each payload decoded by its
declared type (F-379). S7 gains a MAY: a profile orders more strictly.
Observer §4.1 states, without a keyword, that a tool keeps the merge for
as long as it follows, and that one that bounds what it holds counts what
it forgot. 0.68 corrects 0.67's S7 sentence, which round 90's late joiner
followed. zk2py diffed every RFC 2119 sentence of both texts: no other
rule changed.

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level. Not the deployment class.
The profiles' classes, by role, are round 89's table above.

### Its walk of `RULES.md` at 264ea30d

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 56 | 45 | 3 | 8 |
| consumer | 13 | 10 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 52 | 48 | 2 | 2 |
| static, in every class | 34 | 32 | 0 | 2 |

- **168 rows walked, none without a verdict.** The six verdicts keyed to
  moved sentences are re-keyed (S7's two, §8.4's two under caller,
  observer §7.3's two), and §8.4's two new rows under observer have
  theirs (met).
- **Beyond the index:** eleven observer rules stated without a keyword,
  each met: round 90's nine, and observer §4.1's lasting merge and bound.
- **Not met:** the owner's three SHOULDs (§1.4's canonical form, §1.5's
  new session at a re-mint, §3.3's 1 KB), and observer §7.3's two MUSTs,
  in the echo tool alone: the profiles' readers meet 0.10's "each payload
  by its declared type".

### What round 91 changed in zk2py

- **A consumer by S7 from its first value.** `seed.LateJoiner` orders a
  consumer's values by S7's consumer rule from the first, its seed's
  replies included (`seed.Merge`, rule `S7`). A tool's keeps §4.2's merge
  for as long as it follows (rule `merge`). Round 90's switch at the GET's
  final reply is gone; the queue still orders each key's values as they
  arrived.
- **A tool's merge bounded.** `seed.Merge` holds a tool's at most
  `seed.MAX_KEYS` (65,536) keys, the least recently arrived forgotten
  first and counted in `forgotten`, a forgotten key holding nothing.
  `freshness.Subscriber`, the recording the alarms, health and config
  readers merge from, is bounded and counted the same way. A consumer's
  late joiner refuses a bound: observer §4.1's is a tool's, and a key S7
  forgot would take any stamp as its first again.
- **A key the bound forgot reads as heard of since the forgetting.** The
  subscriber cannot tell it from a key never heard, so it dates its
  listening to every key it holds nothing of from its last forgetting. A
  freshness judge then says less, never more: "no delivery" only for a
  silence it held (F-398; review-bot on PR #971).
- **The readers' merge.** The alarms, health and config readers merge
  through `seed.merged`: by time, then by id as `Latest` compares, a delete
  kept by its stamp. Round 90's health and config readers took the
  subscription's last put by arrival, and the alarms reader compared ids
  as integers.
- **`state.md` §7 step 2 inside the seed, live.** A second session's
  non-complete queryable holds each late joiner's GET open after the owner
  answered; the owner restarts with no record and puts again. `Latest`
  delivers the reply at the GET's end (core §4.1), so it arrives last, and
  the consumer keeps it, the old epoch's, whichever clock is ahead
  (F-393). The tool keeps the later time.

**`readings`: 728, up from 717.** **`just py-conformance`: 3685 of 3685.**

**Live, round 91**, by (owner, readers):
- `state_scenarios` §7: zk2py, zk2py **15 of 15**, §7's first run and the
  run inside the seed with the new clock behind and on the host's clock;
  Rust, zk2py (`live_interop --only s7`) **13 of 13**, the first run and
  the run inside the seed on the host's clock.
- the readers' new merge: `health_scenarios` **29 of 29** and
  `config_scenarios` **51 of 51** (zk2py, zk2py); `alarms_scenarios
  --quick` **78 of 78** with zk2py's owner and **78 of 78** with the owner
  example, §3 step 3's 65 s wait not run; `live_interop --only health
  --only config --only seed` (Rust, zk2py) **129 of 129**.
- `presence_scenarios` was not run this round: nothing it reads moved.

Round 91 found F-393 to F-405, thirteen, none a blocker:
- **a gap in a rule:** F-393, a consumer inside a seed holds whichever
  instance answered, since `Latest` delivers every reply last; and F-398,
  what a tool says of a key its bound forgot;
- **ambiguous:** F-397, §7.3's "by its declared type" beside the decode
  order;
- **the texts:** §0's "They are §1's, §9's" beside §12 (F-395), and §8.4's
  sentences that still say a caller alone (F-396);
- **the index:** S7's MUST shown without its new MAY (F-394), and the
  header's MAY "listed only beside a rule it excepts" (F-400);
- **the glossary:** Epoch and Static rule (F-399);
- **five hard to read:** F-401, §4.2's merge MUST; F-402, "there a consumer
  and a tool part"; F-403, "no end to state"; F-404, "the one whose value
  arrived last"; F-405, `RULES.md`'s "So is one of §1 or §9".

## Core 0.69 and `observer.md` 0.11: round 91 answered (round 92)

Core 0.69 and `observer.md` 0.11 answer round 91's F-393 to F-405 (#926's
fourth part). All thirteen are resolved. F-398's second half, a forgotten
key shown as not held, was declined on purpose. zk2py diffed every RFC 2119
sentence of both texts, and every other sentence: no rule changed beyond
what the changelogs state.
- **The core** asks nothing new. §4.2's merge MUST is three sentences, the
  same obligations (F-401). §4.3's new bullet says what S7 already held
  inside a seed: the consumer holds the id of the instance that answered
  (F-393). The rest is wording.
- **The observer** asks two things new, without a keyword, each stated in
  its entry. §4.1's bound reports its count (F-398), and §7.3's decode
  order puts the declared type before the sample's `Encoding` (F-397).

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level. Not the deployment class.
The profiles' classes, by role, are round 89's table above.

### Its walk of `RULES.md` at 9d026529

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 56 | 45 | 3 | 8 |
| consumer | 15 | 12 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 53 | 49 | 2 | 2 |
| static, in every class | 34 | 32 | 0 | 2 |

- **171 rows walked, none without a verdict.** Observer §4.1's bound
  verdict is re-keyed to 0.11's sentence. F-401's split has three new
  rows: two under consumer ("It MUST keep the newer", "a consumer MUST
  instead keep") and one under observer. Each is met.
- **Beyond the index:** twelve observer rules stated without a keyword.
  Eleven are met: round 90's nine, and observer §4.1's lasting merge and
  its bound. Observer §7.3's decode order is walked from this round, not
  met.
- **Not met:** the owner's three SHOULDs (§1.4's canonical form, §1.5's
  new session at a re-mint, §3.3's 1 KB), and observer §7.3's two MUSTs
  and its decode order, in the echo tool.

### What round 92 changed in zk2py

- **A tool's bound reports its count** (observer §4.1, 0.11).
  `seed.bound_note` says how many keys the bound forgot, and what that
  costs. A tool's `LateJoiner.bound_note()` and `freshness.Subscriber.note()`
  give it. The alarms, health and config readings, and
  `freshness.read_service`'s answer (a `ServiceRead`), carry `forgotten`
  beside what they hold. zk2py marks neither a forgotten key nor its next
  value, as the reference does not, so the count is what tells a person.
- **`state.md` §7 step 2 inside the seed, as 0.69 writes it.** The second
  queryable lets the GET end once rev 11 is heard, then the owner writes
  rev 12. The consumer holds rev 10 until rev 12; the tool keeps the later
  time. The owner example runs it with its clock 5 s behind too. The tool
  is judged by the stamps, since "rev 10 with the clock behind" holds only
  while rev 12 comes within the offset of rev 10 (F-408).
- **The echo tool and §7.3's decode order.** It still decodes nothing. It
  lacks a contract per key, the decoders by declared type on the wire the
  `Encoding` picks, honest rendering by the declared type, and sniffing of
  a foreign payload. The profiles' readers read the wire their profile
  names, not the sample's `Encoding` (F-409).

**`readings`: 737, up from 728.** **`just py-conformance`: 3694 of 3694.**

**Live, round 92**, by (owner, readers):
- `state_scenarios` §7: zk2py, zk2py **21 of 21**, the first run and the
  run inside the seed with the new clock behind and on the host's clock.
- `live_interop --only s7`: Rust, zk2py **26 of 26**, the same three
  runs.
- `health_scenarios` (zk2py, zk2py) **29 of 29**, the readers' new field
  at run time.

Round 92 found F-406 to F-413, eight, none a blocker:
- **gaps:** F-406, a zk2 key whose contract the tool does not hold, which
  §7.3 does not place; F-408, `state.md` §7's clock-behind expectations
  need rev 12 within the offset;
- **ambiguous:** F-407, the bound counts keys or forgettings;
- **a conflict:** F-409, §7.3's wire by the `Encoding` beside
  `telemetry.v1` §2.6's resource encoding and `alarms.v1` §2.4's strict
  JSON;
- **four hard to read:** F-410, "the risk lasts the GET"; F-411, "both
  times"; F-412, the decode order's "then sniffing"; F-413, `RULES.md`'s
  "So does a verb".

## Core 0.70 and `observer.md` 0.12: round 92 answered (mend pass)

Core 0.70 and `observer.md` 0.12 answer round 92's F-406 to F-413 (#926's
fifth part). All eight are resolved. This was a mend pass, not a round: no
cold read beyond the two entries, the sections they name and `state.md` §7
step 2. No MUST or SHOULD changed in either text. Three sentences without a
keyword changed what a rule asks: §7.3's payload whose declared type the
tool does not hold (F-406), §4.1's count of each forgetting (F-407) and a
profile's reader (F-409). The rest is wording.

### Its walk of `RULES.md` at c41f57ab

The index's 171 rows, and their verdicts, are round 92's: the table above
stands. Two verdicts beyond the index are re-keyed to their new sentences
(§4.1's bound, §7.3's decode order). Beyond the index are now fourteen
observer rules stated without a keyword, all met: §7.3's decode order moved
to met, and 0.12's two new sentences (F-406, F-409) have their verdicts.
Not met: the owner's three SHOULDs and observer §7.3's two MUSTs.

### What the mend changed in zk2py

- **Sniffing** (observer §7.3, F-406). `decoding.sniff` renders a payload
  whose declared type the tool does not hold: JSON, CBOR, printable text,
  else hexadecimal. `decoding.unheld` says which case it is and where it
  stopped resolving the key: a foreign key at the grammar, a zk2 key whose
  contract is not in hand (or unavailable) at the contract, a control form
  at the form. Every echo tool row carries both.
- **The decode order** (F-412). A held JSON Schema type is read on the wire
  its sample's `Encoding` names, else its contract's `encoding`
  (`decoding.decode_json_type`). `config.v1`'s reader now reads so, its
  profile saying nothing of it. A document that does not decode, or is not
  an object, is named, and every §5 answer that found none says so.
- **The bound's note** (F-407) says forgettings, the count it always kept.
- **`state.md` §7 step 2's premise** (F-408), checked in every run by
  `checks.premise` on the runner's clock, whatever the offset. A run past
  it fails on it. `live_interop`'s run after
  the seed now writes rev 12.

**`readings`: 752, up from 737.** **`just py-conformance`: 3709 of 3709**, after review-bot on PR #994 (751 and 3708 before it).

**Live**, by (owner, readers):
- `state_scenarios --only 7`: zk2py, zk2py **24 of 24**, each run within
  its premise.
- `live_interop --only s7`: Rust, zk2py **30 of 30**, each run within its
  premise.
- `observer_scenarios --only grammar1`, zk2py's echo tool sniffing beside
  each owner, zk2py's and Rust's: **38 of 38**.
- `config_scenarios`: zk2py, zk2py **51 of 51**; Rust, zk2py
  **51 of 51**.

No new finding: none open.

## Core 0.71 to 0.73 and `observer.md` 0.13 to 0.15: the zk2c envelope, runs and seqs (round 93)

#992 and #976's three steps changed the rules since the mend pass:
- **Core 0.71, `observer.md` 0.13** (#992) restate a fact: a read on a
  token's own session can answer that token, not always.
  `redundancy.v1` 0.8's §2.5 step 1 follows. No rule changed.
- **Core 0.72** (#976 A1) defines §7.5's **zk2c envelope**, CH, a 16- or
  32-bit run, a 24-bit seq and the record, with two static rules: its
  refusals in order, and its round trip.
- **`observer.md` 0.14** (#976 A2): a tool reads a stream, state or event
  sample under `zenoh/bytes` as that envelope first, never sniffing a
  refused one.
- **Core 0.73, `observer.md` 0.15** (#976 A3): an owner writes the envelope
  on every stream, state, `@state` and event put and every state delete,
  under a run and a seq that now order a key's values (§4.3, S1–S3, S7,
  §4.2's merge, §3.3's `run`, §8.2's session posture). A Zenoh timestamp
  stays only on §4.3's table. `health.v1` 0.7, `timing.v1` 0.8,
  `redundancy.v1` 0.9 and `desired.v1` 0.12 follow, editorially.

zk2py diffed every RFC 2119 sentence of `core.md`, `observer.md` and the
four profiles against the mend pass's, by section. **No rule changed beyond
what the changelogs state.** One scenario did: `desired.v1` scenarios §6
gained its clock 5 s behind and a stamp expectation, which the 0.12 entry
does not say (F-421).

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level. Not the deployment class. The
profiles' classes, by role, are round 89's table above.

### Its walk of `RULES.md` at a8bda1fa

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 62 | 51 | 3 | 8 |
| consumer | 15 | 12 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 55 | 51 | 2 | 2 |
| static, in every class | 36 | 34 | 0 | 2 |

- **181 rows walked, none without a verdict.** The three removed §4.3
  verdicts (the HLC MUST, the catch-up's MUST NOT, the new epoch's MUST)
  are gone. §1.5's SHOULD, S2 and §4.2's merge are re-keyed to 0.73's
  sentences, and S1 and S3 moved under unchanged keys. The 20 new rows
  were "not yet" in the mend commit, and each is met now.
- **Beyond the index:** sixteen observer rules stated without a keyword,
  all met. Two are new: §4.4's check of the table's Zenoh timestamps alone,
  and §8.3's in-band stamp shown as a run and a seq.
- **Not met:** the owner's three SHOULDs (§1.4's canonical form, §1.5's new
  session at a re-mint, §3.3's 1 KB), and observer §7.3's two decoding
  MUSTs, in the echo tool. Unchanged.

### What round 93 changed in zk2py

- **`zk2c`** (core §7.5, observer §7.3): the envelope's header, parse (the
  six refusals in order, each with its token), write, and which sample
  carries one (`carries`). `read_sample` reads a data sample envelope first:
  its record, its in-band stamp, or the refusal it names, a refused one's
  bytes never handed on. Every reader of a data sample reads through it:
  the late joiner, S4's GET, `freshness.Subscriber` (and so the alarms,
  health and config readers), the echo tool (a row's `envelope` run and seq,
  its `undecodable`), the timing, desired, arbitration and redundancy
  consumers, telemetry's reader, and a role's subscription.
- **The owner** (core §4.3, §3.3, §8.2): an envelope on every stream,
  state, `@state` and event put, and a delete envelope as each state
  delete's attachment, under `zenoh/bytes`. A 16-bit run from
  `run_record`, durable before any write under it, else a 32-bit draw, and
  for a candidate. One seq per run over state and events, and a new run,
  its descriptor first, before 16,777,216 (`set_next_seq`). Each stream key's
  seq its own, wrapping, kept across a takeover and a member removed and
  added (`remove_member`). The descriptor states `run`. A Zenoh timestamp
  only on §4.3's table (`keeps_timestamp`), and timestamping off but for a
  candidate. S2's replies are the bytes the mutation put, and a
  `reply_del` carries the delete's attachment.
- **The merge** (core §4.2, §4.3, observer §4.1): `seed.Mark`, the in-band
  stamp and the Zenoh timestamp, and `seed.relation`. Within one run by seq,
  plain or serial by the key's kind. Across two runs unordered: a tool
  keeps the later arrival, a consumer accepts. A sample with no envelope is
  ordered by its Zenoh timestamp as before, unordered beside an in-band one.
- **The descriptor checker:** D002 for a `run` that is not 4 or 8 lowercase
  hex digits.
- **S1's check** (observer §4.4): `live.s1_check(on_table=False)` asks
  nothing off the table. `live_interop`'s 0.16 run reads a table resource.
- **`state_pairings`**: `state.md`'s §1, §2, §7, §10 and §13 at 0.73, in the
  three pairings. `state_scenarios` runs them for zk2py, its §3 and §12 with
  `--owner rust` too, and the old §7 and §10 code is gone, with
  `live_interop`'s `s1`, `s7` and `seed` runs, which now call it.

**`readings`: 759, up from 752.** **`just py-conformance`: 3878 of 3878**,
the new `envelope` family's 108 among them, `seed`'s 67 with `inband.json`'s
22 orders and 16 cases (9 for a consumer too), and `descriptors`' 54.

**Live, `state.md` whole**, by pairing (owner, readers):

| § | zk2py, zk2py | Rust, zk2py | zk2py, Rust (`zenctl`, `consume`) |
|---|---|---|---|
| 1 | passed, both routers | passed, both routers | passed, both routers |
| 2 | passed | passed | passed (`zenctl get state`) |
| 3 | passed | passed | passed (`zenctl doctor`) |
| 4, 5, 6, 9 | not runnable: an archive | not runnable: an archive | not runnable: an archive |
| 7 | passed, steps 1 to 6 | passed, steps 1 to 6 | steps 1–3 and 6 passed (`zenctl watch`); 4–5 not runnable: no black box seeds |
| 8 | not runnable: a union storage | not runnable | not runnable |
| 10 | passed | not runnable: the owner example cannot hold a GET | not runnable: no black box seeds |
| 11 | passed | no owner: as zk2py, zk2py | not runnable: no black box merges |
| 12 | passed, 200 rounds | passed, 200 rounds | passed, `zenctl doctor` and `storage list`, 200 rounds each |
| 13 | passed | passed (`--standby`, `--timing`) | step 3 passed (`zenctl watch`); 1–2 not runnable: no black box merges |

`state_pairings`: **107 of 107**. `state_scenarios` §3, §11, §12: **12 of 12**
(zk2py), **7 of 7** (`--owner rust`), **13 of 13** (zenctl's readers).
`live_interop --only 0.16`: **7 of 7**; one Rust-owner run over
`zk2py_echo.v1` **39 of 39**. Beside them, with the owners writing the
envelope: `freshness_scenarios` **23 of 23**, `health_scenarios` **29 of
29**, `redundancy_scenarios` **105 of 105**, and `alarms_scenarios` (the
owner example) **80 of 80**.

**Not re-run this round:** the other profiles' live suites (config,
telemetry, desired, timing, arbitration, budget, link, hostid, presence,
observer, security, acl), whose owners now write the envelope and whose
readers read it through `zk2c`. Their checks of a sample's `Encoding` or
Zenoh timestamp off §4.3's table may still say 0.70's. `freshness_scenarios`
§1's did, and `health_scenarios` §6's stand-in archive kept records where
core §4.4 keeps the owner's bytes; both are mended. `desired_scenarios` §6
restarts its author on the host's clock, where 0.12's scenario puts it 5 s
behind (F-421), and its §7 archive is an owner that would wrap what it
holds in an envelope of its own.

Round 93 found F-414 to F-421, eight, none a blocker. See "At core 0.73
and observer.md 0.15" in `SPEC-FINDINGS.md`.

## Core 0.74 and `desired.v1` 0.13: round 93 answered (mend pass)

Core 0.74 and `desired.v1` 0.13 answer F-414 to F-421. They add or remove
no MUST, SHOULD or MAY. F-414 and F-417 settle the scope of §8.2's and
§4.3's rules, and the rest is wording: §3.3's descriptor put, §4.3's
record, "its seq for state and events", `types.md` §1, and `desired.v1`
0.12's change to scenarios §6. zk2py read the two entries and the
sentences they name, nothing else cold.

### Its walk of `RULES.md` at dcd5f968

Round 93's table, unchanged: owner 62 (51 met, 3 not, 8 n/a), consumer 15
(12, 0, 3), caller 13 (13), observer 55 (51, 2, 2), static 36 (34, 0, 2).
181 rows walked, none without a verdict. §4.3's MUST NOT is re-keyed to
"its seq for state and events". Beyond the index are seventeen rules
stated without a keyword, all met. Sixteen are round 93's observer rules,
and the new one is §3.3's descriptor put, which carries a Zenoh timestamp
the owner sets.

### What the mend changed in zk2py

- **Review-bot on PR #1005:**
  - `Owner.take_run` reads a run record it cannot use as no record. It
    draws a 32-bit run and leaves the record as it is, and `start()`
    never fails on it.
  - `live_interop --only seed` is back over the owner example.
  - `zk2c.read_put` and `read_delete` never raise, and a reading shows it.
  - A comment explains `runs_used`; `zk2c` is imported at module level;
    `Report.note` is `info`.
- **`desired.v1` scenarios §6:** the author restarts with its clock 5 s
  behind (`desired_owner --clock-offset-ms`), and its re-put is checked
  above the last stamp it issued.
- **The live suites, off 0.70's assumptions:**
  - a stranger's put (`link`, `acl`, `budget` §3, `timing`'s stray,
    `arbitration`'s raw safety and §11's X) writes §7.5's envelope;
  - a check that read a Zenoh timestamp off §4.3's table now reads the
    in-band run against the descriptor's (`link` §5's seed by
    `seed.Mark`, hostid's S4 reply, `telemetry` §1 and §2, `desired` §5);
  - `Owner.stamp_table = False` is the rig for `timing.v1` §5's owner that
    "sets no stamp on its puts";
  - the `hlc` measurement asks for an HLC session;
  - the echo row carries `envelope` and `undecodable`;
  - `state_pairings`' tap keeps arrival order.
- **`state_pairings` §7 step 3** checks each descriptor put's Zenoh
  timestamp against the owner's zid (F-415).

**`readings`: 768, up from 759.** **`just py-conformance`: 3887 of 3887.**

**Live**, every suite round 93 did not re-run:

| Suite | Pairings and counts |
|---|---|
| config | zk2py 51/51, owner example 51/51, `live_interop` 97/97 |
| telemetry | zk2py 57/57, owner example 56/56, with `zenctl telemetry` 164/164 |
| desired | zk2py → zk2py 192/192 watching and polling; the three pairings with the owner example 86/86 each, both ways |
| timing | `timing_scenarios` 28/28; `timing_interop` 182/182 over four pairings (the Rust owner's on #1009's black boxes, §5 whole) |
| arbitration | 329/329 over four pairings: py:py 83/83, and rs:py, py:rs, rs:rs 246/246 on #1009's black boxes |
| budget | 41/41 on both owners; `live_interop` 16/16 |
| link | 89/89 |
| hostid | 35/35; `live_interop` 28/28 |
| presence | (zk2py, zk2py) 95/95, (Rust, zk2py) 89/89, (zk2py, zenctl) 33/33, (Rust, zenctl) 32/32 |
| observer | 50/50, both owners |
| security | §4 33/33, both blocks |
| acl | `live_interop` 31/31 |

The mend found one new finding, **F-422**, a contradiction. In
`arbitration.v1` scenarios §11, X's "raw" puts carried no envelope under
core 0.73, so every reader refused them and no `not_incumbent` count
could grow. zk2py took reading (a): X keeps the envelope. #1009 resolved
it in the reference, whose `commander raw on` and `timing stamp none` now
seal their records. With those black boxes §11 passes whole, its known
marks are gone, and `timing_interop` §5 runs with the owner example, its
control on `--hlc`. See "At core 0.74 and desired.v1 0.13" in
`SPEC-FINDINGS.md`. The observation of a forged reply on a wildcard GET
sits beside the findings.

## Core 0.75: the `packed` type kind (round 94)

Core 0.75 (#979 B1, PR #1012) adds a type kind whose record is bit fields
sized from the contract's declared ranges (§7.6), and the tag it gives
§7.5's envelope. Authoring draft 3 lists packed files under `[schemas]
packed`, and a reference reads `packed:Name` or `packed:stem#Name` (§9.1,
§9.4). Eight lints, E038 to E045, and cascade 7 read a packed file (§9.2).
Bundles, retention and the classifier take the kind in (§9.5 to §9.8).
`observer.md` and the profiles did not change.

zk2py diffed every RFC 2119 sentence of `core.md` between 0.74 and 0.75, by
section. **No rule changed beyond what the changelog states,** and no
existing fixture's expectation moved.

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level, and not the deployment class.
§7.6's rules are static, so they are in every class.

### Its walk of `RULES.md` at 8e810d52

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 62 | 51 | 3 | 8 |
| consumer | 15 | 12 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 55 | 51 | 2 | 2 |
| static, in every class | 45 | 43 | 0 | 2 |

- **190 rows walked, none without a verdict.** §7.1's uniqueness MUST, now
  two rows, is re-keyed to "the listed JSON Schema files" beside its packed
  twin. The nine new static rows (§7.1's packed uniqueness and E044, §7.6's
  six, §9.4's packed stems and ids) were "not yet" in the walk's commit,
  and each is met now.
- **Beyond the index:** round 93's seventeen, all met.
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's two
  decoding MUSTs.

### What round 94 changed in zk2py

- **`packed`** (core §7.6, §9.2):
  - a packed file read and linted, E038 to E045 under cascade 7: E043 before
    E038, one of E038 and E043 per field, one of E039, E040 and E041 in the
    cascade's order, E042 and E045 over the fields with neither;
  - each type's widths, item offsets, shortest and longest record, layout
    form, its sha256 and tag;
  - the writer: exact decimals over `fractions.Fraction`, a tie to the even
    code, reserved codes from all ones down in §7.6's order, a counter
    modulo 2^width, a time from the deployment epoch, and the vector's
    refusals (`unknown`, `missing`, `value`, `range`, `epoch`);
  - the reader, refusing in §7.6's order (`tag`, then field by field
    `length` and `code`, `extension`, `length`, `pad`), a marker read as 1
    at its layout's end reading the rest as a later revision's;
  - `seal` and `open_put`: a record in §7.5's envelope, CH's tag bits the
    type's tag, and read back under the tag CH carries.
- **`schemas`:** `[schemas] packed` loaded after the JSON Schema files,
  E024 for a taken packed stem and for an id either kind took, E028 and
  E029 at load, E029 suppressing a packed reference's E023; `packed:Name`
  and `packed:stem#Name` resolved, E023 and E024.
- **`contract`:** E044 on each resolved packed reference that is an explicit
  stream's type, an attachment or an operation's type; W103 on a packed
  type.
- **`bundle`**, **`compat`:** a packed document carried as a JSON Schema
  one, hashed and kept by its JCS bytes; §9.8's packed rules field by
  field (`fields_changed`, `type_changed`, `presence_changed`,
  `bound_changed`, `resolution_changed`, `width_changed`,
  `reserved_changed`, `enum_changed` breaking; `field_renamed`,
  `enum_value_renamed`, `enum_value_removed` review; growth behind a
  marker compatible).
- **The runner:** a `packed` family over `packed/vectors.json` (13 layouts,
  48 writes, 34 reads; each accepted write sealed, opened and read back, and
  refused under another tag; each read through the envelope too); the
  packed bundle built from `contracts/ok-packed.toml` byte for byte, and its
  tampered twin one packed document apart; transitive packed revisions
  wrapped as `packed`.
- **`readings`:** round 94's eleven findings, each sentence as it reads and
  the reading zk2py takes, shown through `packed` and the lints.

- **Review-bot on PR #1014:** the writer refuses `unknown` and then
  `missing`, over the whole layout, before any bit (E045 keeps a field not
  optional after a marker out of a clean type, and a `Record` built
  without the lints is refused too); a writer's decimal is read with work
  the declaration bounds, a magnitude no code reaches refused `range`
  before any arithmetic and digits past the field's ties truncated with a
  sticky digit, exactly; a time's range compared before its arithmetic;
  `true` refused for an `int`, a `counter`, a `time` and a `fixed`; E044
  checked on a state's and an event's attachment and an operation's four
  types; every accepted write refused under each of the 15 other tags.

**`readings`: 785, up from 768.** **`just py-conformance`: 4136 of 4136**,
`packed` 193 among them, `contracts` 126 (12 packed fixtures, two with
canonical bytes), `bundles` 28 and `compat` 120 (20 packed).

**The black box.** `zk2 contract lint` and `zk2 contract fingerprint`,
built from main plus #1009, refuse every packed fixture with E000
(`unknown field packed`): B1's model is not in them. On the other 87
contract fixtures their codes (W107 aside) and fingerprints are zk2py's.

**Not run:** anything live. No runtime writes a packed record yet (B2), and
no tool reads one.

Round 94 found F-423 to F-433, eleven, none a blocker. The vectors decide
F-423 to F-425 and F-430, and zk2py follows them. See "At core 0.75" in
`SPEC-FINDINGS.md`.

## Core 0.76 and `observer.md` 0.16: round 94 answered (mend pass)

Core 0.76 and observer text 0.16 (#979 B2, PR #1016) answer F-423 to
F-433. zk2py read the two entries and the sentences they name, and the
vectors and fixtures they add, nothing else cold. A keyword diff of
`core.md` and `observer.md` between 0.75 (99cce33d) and 69af15a6 finds
§7.6's reading MUST reworded ("naming the refusal", a colon before its
list) and one new MUST NOT, the writer's `unknown` and `missing`.
`observer.md`'s keyword sentences are unchanged, though §7.3's decoding
MUST now lists `packed`. **No rule changed beyond what the changelogs
state.** Since #1016 a Rust runtime writes and reads packed records, so
they ran live for the first time.

### Its walk of `RULES.md` at 69af15a6

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 62 | 51 | 3 | 8 |
| consumer | 15 | 12 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 55 | 51 | 2 | 2 |
| static, in every class | 46 | 44 | 0 | 2 |

- **191 rows walked, none without a verdict.** The new row, §7.6's MUST NOT
  on an object holding a member its type does not have or lacking a field
  that is not optional, is met.
- **Beyond the index:** twenty-one, all met. Four are new: observer §7.3's
  packed record read under its CH tag, its epoch (0 where none is stated)
  and its name `packed:<name>`, and core §7.6's E = 0.
- **Not met:** unchanged. Observer §7.3's decoding MUST now names
  `packed`, which zk2py's echo tool decodes, but not protobuf or JSON
  Schema field by field.

### What the mend changed in zk2py

- **`packed`:** an `int` bound past ±(2^53 − 1) goes into the layout
  form's JCS as RFC 8785 writes a double (`Wide64`'s tag). An `int`,
  `counter` or `time` value written with a fraction or an exponent is
  `range` at a magnitude of 2^63 or more and `value` below it, while
  integer text is read by its exact value (F-434). `deployment_epoch` is
  0 where a deployment states none (F-433), and `read_sample` reads a put
  as observer §7.3 (0.16) does: under CH's tag, and undecodable with the
  refusal named when the layout refuses it or it came in no envelope.
- **`zk2c.Read`** carries CH's tag.
- **The owner:** CH's tag bits are a packed type's tag on every stream,
  state and event put (`Owner.tag_of`), and `Owner.encode_packed` writes
  a value under the deployment epoch.
- **The consumer:** `Owner.subscribe_role(record=…)` reads each put
  through the layout of the revision it holds, and counts a refused one
  in `undecodable` with its token.
- **The echo tool:** `--contracts` (one revision per interface) and
  `--epoch`. A packed record is `decoded`, or `undecodable` with the
  refusal named, never sniffed, and `declared_type` names `packed:<name>`.
- **`packed_live`:** the two pairings below, over the Rust tests'
  `packed.v1` and its `packed-wide` and `packed-collide` revisions, with
  `interop/packed/zk2py_packed_reader.v1.toml` as the consumer's contract.
- **`readings`:** round 94's sentence pins read 0.76's answers, the 0.75
  forms gone. Also new: #1012's three cases, zk2py's readings of F-434 and
  F-435, and the echo tool's packed rows.
- **Review-bot on PR #1019:** a reading pins `zk2c.Read`'s three cases
  (accepted, refused, no envelope) through `read_sample`. The echo tool
  names a refused envelope under a packed type as that type's
  (`undecodable as packed:<name>: <token>`). `subscribe_role` hands a
  delete's sink `None` under a packed record, never bytes. The 2^63
  comment says the cut is a float's alone, and `packed_live` waits on
  signals.

**`readings`: 801, up from 785.** **`just py-conformance`: 4202 of 4202**,
`packed` 242 among them (15 layouts, 63 writes, 43 reads) and `contracts`
127.

**The black boxes, offline** (main 69af15a6). `zk2 contract lint` and
`zk2 contract fingerprint` know packed now. On the 13 packed fixtures and
the Rust tests' three packed contracts, their codes (W107 aside) and
fingerprints are zk2py's, and both give E028 for a contract listing
`Wide64`.

**Live** (`python -m zk2py.packed_live`):

| Pairing | Result |
|---|---|
| rust-py: `zenctl gen` writes, zk2py reads | **12 of 12.** Each step waits on a signal (the tap hearing the owner, the tool's "listening"), not a sleep. About 510 records on 14 streams, 2 states and an event read through zk2py's layouts, and zk2py's writer writes each reading back to the same bytes. The Rust clock records read as now under E = 0. S4's GET reads both states. zk2py's consumer and echo tool refuse the records zk2py put on the sensor key itself: by `tag` (`SensorWide`), by `length` (`SensorCollide`), and with no envelope. Consumers holding `packed-wide` and `packed-collide` refuse every Rust sensor record by `tag` and by `length`. |
| py-rust: zk2py's owner writes, Rust reads | **13 of 15.** `zenctl watch` is waited for by a publisher's matching status, and `zenctl echo` by its "echoing" line. `zenctl get state`, `zenctl watch` (four streams and the event; the clock under `epoch_assumed` 0) and `zenctl echo` show zk2py's packed fields. Holding `packed-wide` or `packed-collide`, `zenctl watch` delivers none of zk2py's sensor records. The two failures are F-436. `zenctl watch` drops three records the `Ties` layout refuses, with no row and no count. `zenctl echo` names each refusal, but renders a one-byte record as the text `"\u0012"`. |

The mend found three findings. **F-434** (contradiction): "a number of
magnitude 2^63 or more is outside every range, however it is written",
beside the counter's 0 to 2^64 − 1 and the vectors' write 21. **F-435**
(contradiction): the vectors' write 46 still refuses `epoch` where 0.76
gives E = 0. **F-436** (bug): the reference's `zenctl watch` and `zenctl
echo` beside observer §7.3's honest rendering. See "At core 0.76 and
observer 0.16" in `SPEC-FINDINGS.md`.

## Core 0.77 to 0.80 and `observer.md` 0.17 and 0.18: the catalog and keys across a face (round 95)

Four core versions and two observer texts landed since the mend after
round 94:
- **core 0.77 and observer 0.17** (#1011) label zenoh's defaults as
  1.10.1's, beside the fork `zm-1.10.2-r1`'s, and add a stall bound to
  `budget/lossless.json`;
- **core 0.78 and observer 0.18** (#980 C1b) add §8.5's "Keys across the
  face", its tool's half in observer §7.5, and Appendix B's facts on how
  zenoh 1.10.1 shortens a put's key, measured by `constrained.md` §7;
- **core 0.79** (#980 C1a) adds §8.6's deployment catalog, §3.3's
  descriptor `catalog`, and §7.6's epoch, the catalog's else 0;
- **core 0.80** (#1021, on `fix-packed-followups`) answers F-434 and
  F-435.

zk2py diffed every keyword sentence of `core.md` and `observer.md` between
69af15a6 and af59a1ae, and read 0.80's diff. None is gone or reworded,
and each new one is its changelog's. **No rule changed beyond what the
changelogs state.** `lossless.json`'s new case, the fork's 10 s
`wait_before_close` beside a 10 s lease, gives 7.5 s and passes.

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level, and not the deployment
class. §8.6's rules that name no role are static, so they are in every
class.

### Its walk of `RULES.md` at 0.80

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 63 | 52 | 3 | 8 |
| consumer | 17 | 14 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 57 | 53 | 2 | 2 |
| static, in every class | 49 | 47 | 0 | 2 |

- **199 rows walked of `RULES.md`'s 201, none without a verdict.** The
  eight new rows were "not yet" in the walk's commit, and each is met
  now: §3.3's catalog id (owner), §8.5's two SHOULDs (consumer), §8.6's
  reader and observer §7.5 (observer), and §8.6's reading, id and
  succession (static). §7.6's E = 0, beyond the index, is re-keyed to
  0.79's "Where none is, E is 0".
- **Beyond the index:** twenty-three, all met. Two are new: §8.6's owner
  counting its `time` fields from its catalog's epoch, and observer §8.2's
  "Across a constrained face the three are too wide".
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's two
  decoding MUSTs.

### What round 95 changed in zk2py

- **`catalog`** (core §8.6): `read` reads `zk2-catalog/1` strictly, each
  number kept as its text, and refuses in §8.6's order. `catalog_id` is
  the first 4 bytes of the sha256 of its JCS bytes. `succession` refuses
  `epoch_moved`, `reassigned`, `removed` and `lineage`, one step only.
  `resolve` reads an index or the epoch through the named catalog, or the
  first held whose lineage holds it, below that item's count:
  `unknown_catalog`, `unknown_index`.
- **The descriptor checker:** §3.3's `catalog`, D002 for any spelling but
  8 lowercase hex digits, and the schema's D000 for `null`.
- **The owner:** `Owner.catalog` states its id in the descriptor, and
  `Owner.writing_epoch` dates packed `time` fields from its epoch.
- **`face`** (core §8.5, observer §7.5): a resource's scope, its exact key
  or its template's selector, a rest parameter `*/**` and an event's ULID
  chunk `*` (F-438), a parameter its role binds its value (F-439).
  - The consumer: `Owner.role_keys` subscribes a template named with its
    parameters (`stream/axis/{axis}`) by that selector, nothing wider.
  - The tool: `echo_tool --across-face SYSTEM/SERVICE` with `--contracts`
    subscribes by the contracts' scopes, a subscriber each, refused beside
    a SELECTOR. Its scope names what it excludes: `@op`, `@zk`, `@adv` and
    every key no held contract declares there.
- **`echo_tool --catalog FILE`:** the catalog's epoch for packed `time`
  fields, a refused catalog or one beside `--epoch` refused input, exit 2.
- **`packed`** at core 0.80: integer text from −2^63 to 2^64 − 1 read
  exactly, any other number a binary64 (`json_number`). The codec, handed
  no epoch, writes and reads under E = 0, and the writer's `epoch` token is
  gone (F-434, F-435 resolved).
- **The runner:** a `catalog` family over `catalog/`'s three files, 123
  cases.
- **`readings`:** F-427's pin reads 0.80's sentence, and F-434 and F-435
  read as resolved. Round 95's own pin the sentences F-437 to F-441
  quote, and show zk2py's readings through `face`, `catalog`, the owner's
  writing epoch and the echo tool's refusals.

**`readings`: 813, up from 801.** **`just py-conformance`: 4351 of
4351**, `catalog` 123 among them, `descriptors` 58 and `packed` 250.

**Live** (`python -m zk2py.constrained_live`), `constrained.md` §7 with
zk2py alone, three runs: one router (timestamping disabled, core §8.2),
zk2py's owner at `site/rig` its client, and each step's far client
through `CountingProxy`, a TCP relay that reads the bytes toward the far
client as zenoh's batches. Each put's record is 8 bytes. Bytes are a
batch's, without the TCP framing's two:

| Step | Subscription | A put | Its key's text |
|---|---|---|---|
| 0 | the owner given a catalog | descriptor states `0d389b8e`, no D code | |
| 1 | `…/stream/level`, exact | **25 B** | none |
| 2 | `…/stream/axis/*`, the template's selector | **31 B** | `/roll` |
| 3 | `zk2/**` | **61–62 B** | its 36 B after `zk2`, and a length byte |
| 4 | `…/state/mode`, exact, then a GET | 27 B a put, **87–91 B the reply** | the reply's whole key |
| 5 | zk2py's tool, `face.scopes` at `site/rig` | 25, 31 and 27 B | none past the scope |
| 6 | zk2py's consumer bound to `site/rig` | 25 and 31 B | none past the scope |
| 7 | zk2py's consumer bound to `site/*` | 57 and 61 B | all after `zk2/site` (F-439) |
| 8 | `…/stream/path/*/**`, `…/events/alarm/*` | 30 B, 55 B | `/a/b`; the `/<ulid>` (F-438) |

- **§7's four expectations hold,** and step 3 exceeds step 1 by the key's
  text after `zk2` and its length byte. A batch's frame carries its
  sequence number as a varint, so a batch differs by a byte between
  sessions, as the reference's 45–46 B do. zk2py's payload is smaller than
  the reference's, so its bytes differ by a constant.
- **§8.5's two SHOULDs are met** by zk2py's consumer through an exact
  binding (step 6), and **observer §7.5's** by its tool (step 5), each at
  the steps' own sizes. A wildcard binding cannot meet the first without
  presence (step 7, F-439).

**The black boxes** (69af15a6) predate core 0.77: their `zk2` knows no
`catalog`, and their `zenctl` takes no `--across-face`, so nothing of
this round ran against the reference.

**Not run:**
- `constrained.md` §7 against the reference, and F-436's fix in #1021,
  for the same reason;
- `budget.md` §1 again: its routers now state `wait_before_close` 5 s,
  where zk2py's run 1.10.1's 5 s default without stating it;
- §8.5's fourth step, a router-declared prefix lost on a lossy link, which
  needs #989's lab, as 0.78 says.

Round 95 found F-437 to F-441, five, none a blocker. See "At core 0.79
and observer 0.18" in `SPEC-FINDINGS.md`.

## Core 0.81 and 0.82, `freshness.v2` 0.1, `link.v1` 0.11 and `link.v2` 0.1: the heartbeat (round 96)

Two core versions, a new profile and two link texts landed since round 95:
- **core 0.81** (#978 F1) defines §7.5's form 6, the heartbeat: its tag,
  its collections, its body and the digest, with `heartbeat/`'s fixtures;
- **core 0.82** (#978 F3) adopts it: §4.3's table splits row 1, S2's
  replies carry a member's last confirmation, §4.2's "A heartbeat is no
  mutation", §8.1's tokenless set and §8.5's liveness across a face;
- **`freshness.v2` 0.1** says who sends a heartbeat, how often, and §2.5's
  rule a subscriber reads it by, with `apply.json` and `scenarios-v2.md`;
- **`link.v1` 0.11** lets a heartbeat cross where a collection it covers
  does, never downsampled, and lists it as liveness evidence;
- **`link.v2` 0.1** (#988 L1 and L2) puts the face's transport in the
  face policy, read cold this round.

zk2py diffed every keyword sentence of `core.md` between c29cca1e (0.80)
and 1d1ce925 (0.82). None is gone; the seven new or reworded are their
changelogs'. `observer.md` is unchanged at 0.18. **No rule changed beyond
what the changelogs state**, though 0.82's entry miscounts `RULES.md`
(F-442).

### The classes zk2py claims

Unchanged since round 89: the **owner**, **consumer**, **caller** and
**observer** classes, each at the full level, and not the deployment
class. It now follows `freshness.v2` (owner, consumer and tool) and
`link.v2` (tool).

### Its walk of `RULES.md` at 0.82

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 64 | 53 | 3 | 8 |
| consumer | 17 | 14 | 0 | 3 |
| caller | 13 | 13 | 0 | 0 |
| observer | 57 | 53 | 2 | 2 |
| static, in every class | 54 | 52 | 0 | 2 |

- **205 rows walked of `RULES.md`'s 207, none without a verdict.** The six
  new rows were given verdicts first, not met, in the walk's own commit,
  and each is met now: §7.5's five static heartbeat rules and S2's
  freshness.v2 sentence (owner).
- **Beyond the index:** twenty-four, all met. One is new: §4.2's "Under
  `freshness.v2` an owner puts no member again to confirm it" (owner).
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's two
  decoding MUSTs.

### What round 96 changed in zk2py

- **`zk2c`**: form 6 parsed on a put, its tag reported and its body the
  record, refused as a delete's attachment (`sample_kind`); `Read.form`.
- **`heartbeat`** (core §7.5): the covered revision's tag and collections
  (`layout_of`), the body written (`write_bits`, the list only where it is
  shorter, a tie the bitmap) and read in §7.5's order of refusals
  (`short`, `code`, `trailing`, `pad`), every value written back in the
  form read, and the digest (`member_value`, `digest`).
- **`freshness_v2`** (`freshness.v2` §2.5): `View`, the rule's four steps,
  a gap in the heartbeat key's seqs, two divergences in a row due a read,
  and a read's answer merged as a late joiner's seed; `Subscriber`, the
  same on a session, re-reading by S4's GET, an optional seed of its own,
  and `freshness.v1` §2.5's judgement from the last confirmation, a gap
  read as a lost delivery.
- **The owner** (§2.4, core 0.82): an interface whose contract uses
  `freshness.v2` and has a collection is covered. The owner implements
  the standard contract beside it, added when not given, and sends a
  heartbeat at 0.95 of half the shortest horizon from its tokens on,
  never from a standby, until it closes or steps down. Each map bit is
  its collection's exposure, each digest over the members held live, and
  a bit per member it vouches for (`Owner.vouch`). Under the clock guard
  every vouched bit is clear and the time unknown (F-447). A covered
  member is never re-put, its puts carry no Zenoh timestamp, and its
  replies carry its last confirmation (F-445, F-446).
- **`link`** (0.11): `heartbeat_of`; a heartbeat crosses exactly where a
  collection of its interface does, carved out of `freshness.v2`'s stream
  deny as `heartbeat/<I>`, never downsampled, and its services' liveness
  evidence; an inline revision may be `{uses, resources}` (F-450).
- **`link_v2`** (0.1): the policy (a policy with no `profile` read as
  `link.v1`'s), its binding's link protocols, `[transport]` and each
  face's medium keys with the eight new codes; the plan with the binding,
  the catalog, the transport and each face's reply timeout; the transport
  block at zenoh's keys, and §2.19's checks (F-449).
- **The runner:** a `heartbeat` family over `heartbeat/`'s three files,
  117 cases; `apply.json` in `freshness`, 64 checks; `link`'s two new plan
  cases and `conformance/v2/`'s 52; the envelope vectors' form 6.
- **`readings`:** round 96's pin the sentences F-442 to F-450 quote, and
  show zk2py's readings of them.

**`readings`: 822, up from 813.** **`just py-conformance`: 4607 of 4607**,
`heartbeat` 117, `envelope` 115, `freshness` 166 and `link` 226 among them.

**Live** (`python -m zk2py.freshness_v2_scenarios`), `scenarios-v2.md`
§1–§7 against the black boxes of main 1d1ce925, in three pairings:

| Section | zk2py owner, zk2py reader | Rust owner, zk2py reader | zk2py owner, zenctl |
|---|---|---|---|
| §1 the cadence, and the bits | 9/9 | 9/9 | 8/8 |
| §2 a vouch withdrawn goes stale | 6/6 | 6/6 | 6/6 |
| §3 a GET reader's age | 3/3 | 3/3 | 3/3 |
| §4 a lost change, re-read | 4/4 | 4/4 | 2/2 |
| §5 a late joiner by S4 | 2/2 | 2/2 | 1/1 |
| §6 a restart, a new run | 3/3 | 3/3 | 1/1 |
| §7 across a face | 4/4 | 4/4 | 4/4 |

- **Digests and bodies agree** across implementations: zk2py's reader
  confirms the Rust owner's members from its heartbeats, and zenctl's
  `confirmation` rows confirm zk2py's.
- **The owner example exposes `route`**: it takes no capability argument,
  so §1's map reads 1 1 1 against it, the map its exposure's.
- **zenctl** judges no freshness: S's verdicts apply `v2.md` §2.5 to the
  arrivals of its `sample` and `confirmation` rows, and G's §2.6 to `get
  state`'s `last_confirmation`. Watching the heartbeat resource, it reads
  each heartbeat `undecodable` (F-444).
- **§7** runs `beacon-face.v2.toml`, a revision §7 implies and does not
  write (F-443).

**Not run:**
- `link.v2`'s scenarios §1, a router over a relay counting keep-alives;
- F-436's fix, against the new black boxes.

Round 96 found F-442 to F-450, nine, none a blocker. See "At core 0.82
and freshness.v2 0.1" in `SPEC-FINDINGS.md`.

### Extended to core 0.83: frames, K1's static rules (round 96)

Main moved to baa15f11 before round 96 went up. **Core 0.83** (#981 K1)
defines frames: §7.7, form 5 in §7.5 with `key_kind`, §4.3's seq per
record, authoring draft 4's `frame` with E046 to E050 and cascade 8,
§9.5's `frame` member and §9.8's rows. With it come `timing.v1` 0.9 §2.3
and `arbitration.v1` 0.6's `framed` check. The branch was rebased on it,
cleanly. zk2py diffed every keyword sentence of `core.md` between 0.82
and 0.83. None is gone, the nineteen new ones are §7.7's, and §7.5's list
gains `key_kind`. **No rule changed beyond what the changelogs state.**

**Its walk of `RULES.md` at 0.83:**

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 70 | 53 | 3 | 14 |
| consumer | 20 | 15 | 0 | 5 |
| caller | 13 | 13 | 0 | 0 |
| observer | 60 | 54 | 2 | 4 |
| static, in every class | 61 | 59 | 0 | 2 |

- **224 rows walked of `RULES.md`'s 223, none without a verdict.** A row
  under two classes is walked in each.
- **§7.7's seven static rules are met** by `frame`.
- **Its six owner rules are n/a**, and two of its three reader rules: K2
  and K3 bring the readers and the owners. zk2py's owner refuses a
  contract with a frame resource before it declares anything, and every
  packed reading counts a frame undecodable, `no_reading`. The third
  reader rule, never an unknown or absent field as a value, is met:
  no zk2py reader shows a frame's field.

**What it changed in zk2py:**
- **`zk2c`**: form 5 defined on a put, refused as a delete's attachment;
  `key_kind` for a frame on a key whose kind token is not `stream`, read
  from the key alone; `reads`, `carries.json`'s.
- **`frame`** (§7.7), on `packed`'s fields: the declaration and its
  defaults, the frame form and its tag, the count's and the offset's
  widths, the body written (`records`, `unknown`, `missing`, §7.6's
  `value` and `range`, `order`) and read (`tag`, then `length`, `code` and
  `order` as the fields are read, `length` and `pad`), every value written
  back.
- **The linter**: `frame` a resource's own, E046 to E050 in cascade 8's
  order, and `frame` in the canonical form of a frame resource alone;
  `compat`: `frame_toggled`, `frame_changed`, `deadband_changed` and
  `keyframe_changed` (F-451).
- **`arbitration`**: check 6 counts no frame resource, check 9 `framed`.
- **The runner:** a `frame` family, 120 cases; the envelope's form 5 and
  `reads`, 141; `contracts` 136, `compat` 134, `arbitration` 124.
  **`readings`: 827. `just py-conformance`: 4786 of 4786.**

**Not run:** frames on a bus. No owner writes one at 0.83, and
`scenarios/frames.md`, which §7.7 cites, does not exist yet (F-454).
Round 97 runs them (below).

The extension found F-451 to F-454, four, none a blocker. See "At core
0.83" in `SPEC-FINDINGS.md`.

## Core 0.84 to 0.87, observer 0.20 and 0.21, `link.v1` 0.12 and 0.13, and the profiles onto `freshness.v2`: frames read and written (round 97)

Since round 96 main took:
- **core 0.84** (#980 C1c), the owner's catalog in `presence.md` §2, no
  rule changed;
- **core 0.85**, observer 0.20, `freshness.v2` 0.2, `link.v1` 0.12,
  `link.v2` 0.2 and `timing.v1` 0.10: round 96's F-442 to F-454 answered;
- **observer 0.21** (#981 K2): §7.6, a tool's frames, with
  `frame/apply.json`;
- **core 0.86** (#978 F4): §11.1's Consume grants each bound provider's
  heartbeat; and `health.v1` 0.8, `alarms.v1` 0.11 and `config.v1` 0.9
  onto `freshness.v2`, their contracts' revisions 1.1 published, with
  `alarms.v1` §2.15's `window_unconfirmed` and §2.14 step 5;
  `freshness.v2` 0.3 is draft;
- **core 0.87** and **`link.v1` 0.13** (#981 K3): frames written,
  `scenarios/frames.md` §1 to §6, §7.7's never a mutation, its MAY to end
  a frame early and its "History", W108, and a frame never downsampled.

zk2py diffed every keyword sentence of the changed texts between 7d58513b
and 13cd65c4. **No rule changed beyond what the changelogs state** (0.87's
entry says "History" states no keyword, though it opens with a MAY;
`RULES.md` lists no MAY, so its counts stand). Each of round 96's answers
holds, as its reading shows.

### The classes zk2py claims

Unchanged: the **owner**, **consumer**, **caller** and **observer**
classes, at the full level. `freshness.v2` now covers zk2py's `health.v1`,
`alarms.v1` and `config.v1` owners and readers.

### Its walk of `RULES.md` at 0.87

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 75 | 64 | 3 | 8 |
| consumer | 21 | 17 | 0 | 4 |
| caller | 13 | 13 | 0 | 0 |
| observer | 83 | 79 | 2 | 2 |
| static, in every class | 62 | 60 | 0 | 2 |

- **228 rows walked of 228, none without a verdict.** The walk was
  re-keyed first, in its own commit: S2's re-worded `freshness.v2`
  sentence (met), §7.7's never a mutation (n/a until the owner wrote
  frames), observer §7.3's heartbeat (not met: the echo tool sniffed its
  body) and §7.6's two (n/a). Each is met now, with §7.7's six owner rules
  and its three reader rules, in the consumer and observer classes.
- **Beyond the index:** 26. New: §7.7's MAY to end a frame early (met)
  and "History"'s late joiner (n/a: zk2py's owner publishes no stream
  with history).
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's
  two decoding MUSTs.

### What round 97 changed in zk2py

- **`frame.Reader`** (core §7.7, observer §7.6): one state per field of a
  frame key, `known`, `within` with its delta, `unknown` with why
  (`first`, `run`, `gap`, `rewind`, `deadband`, `no_value`) or `absent`;
  every field `unknown` at a gap, a rewind and another run; a deadband
  field `unknown` at its `max_records`-th record; no value shown for an
  unknown or absent field; a mutation, a heartbeat or a refused body on a
  frame key undecodable, named, its seqs uncounted.
- **`frame.Framer`** and **`Owner.record_frame`**, `end_frame`,
  `lose_frame`: an owner's records gathered into frames, put at `records`
  records, at the span, or early (the MAY); a run's first frame full, a
  full record every `keyframe_every`, deltas of what moved, a deadband
  field omitted only while valued, within its delta and under
  `max_records`, a record scheduled only within `jitter_ms`; record 0 the
  key's next seq, the seq advanced by the count; open frames put before a
  new run. `publish` and `stream_envelope` refuse a frame key.
- **`frame_live`**: `FrameSubscriber`, the consumer of an owner's frame
  keys, and `RawFrames`, `frames.md` §4's X.
- **The echo tool**: a held frame resource's frames read through one
  reader per key, each record with whose clock its time is on; an unheld
  one by its envelope; a heartbeat by its covered revision's collections
  or its envelope (form, interface, size); neither ever sniffed.
- **The linter**: W108 (cascade 8); E050 reads a declaration (F-452's
  answer); `compat` compares a deadband by its field's place (F-451's).
- **`link`** (0.13): a frame resource's interval, largest frame and
  history read from its contract; no downsampling rule for one;
  `frame_over_interval`, `frame_over_batch`, `history_on_frame`, and
  `interval_over_horizon` by its interval. `freshness.v2` a listed
  interface wherever a listed interface has a heartbeat (F-455).
- **`acl`** (core 0.86): Consume grants each bound provider's heartbeat
  where the binding consumes a `freshness.v2` collection, and each covered
  owner's heartbeat joins the `allow` posture's keys.
- **`health.v1`, `alarms.v1` and `config.v1`** on their revisions 1.1:
  their owners confirm by heartbeat, never by a re-put; a closed writer
  clears its member's bit and a reopened one sets it; an alarm confirmed
  again sets its bit and puts nothing; a resolved alarm's bit is clear
  from its resolved put; a `reply_del` of a covered member carries the
  timestamp minted at its delete. `freshness.Subscriber.follow` reads the
  owner's heartbeats by `freshness.v2` §2.5, each confirmation a delivery
  of the member's last put, a gap read as a lost delivery, a complete GET
  a read of the view. `AlarmsReader` follows an owner whose descriptor
  names a revision using `freshness.v2` (read from a history root, else
  retrieved), and its window needs the latest heartbeat in step
  (`window_unconfirmed`); §2.14 step 5 attributes no subscription
  delivery of such an owner.
- **The runner:** `frame/apply.json` (195 checks: each frame's body, its
  reading from its value and as a sealed put), `link`'s
  `conformance/contracts/`, the `in_step` cases, and a published
  revision's review accepted by its publication (F-456). `beacon.v2` is
  the scenarios' 0.2 listing; round 96's `beacon-face.v2` is gone.
- **`readings`: 840, up from 827**: round 96's readings re-keyed to the
  answers, and round 97's thirteen.

**`just py-conformance`: 5018 of 5018**, on main 7597efb8: `frame` 315,
`link` 230, `alarms` 320, `contracts` 138, `compat` 135, `readings` 840,
and `telemetry` 283. Main moved during the round to add `telemetry.v2` 0.1
(#983 T1): its `conformance/v2/` and `conformance/contracts/` are keyed
present and not read, and the walk lists it as not followed; #983's own
round reads it, and round 97 files nothing on it.

### Live

`python -m zk2py.frames_scenarios`, `frames.md` §1 to §6 against the black
boxes of main 13cd65c4, over `linkframes.v1` as `frames.md` writes it:

| Section | zk2py owner, zk2py reader | Rust owner, zk2py reader | zk2py owner, zenctl |
|---|---|---|---|
| §1 each frame decodes alone | 12/12 | 12/12 | 12/12 |
| §2 a lost delta frame | 6/6 | 6/6 | 6/6 |
| §3 scheduled and measured time | 8/8 | 8/8 | 7/7 |
| §4 the deadband | 9/9 | 9/9 | 7/7 |
| §5 a new run | 4/4 | 4/4 | 4/4 |
| §6 a late joiner | 4/4 | 4/4 | 4/4 |

- **The owner example** loses a frame on demand (`frame lose`), so every
  section ran against it.
- **zenctl** reads frames as zk2py does, field for field: `watch` for S,
  and `get` for §1 step 5's frames read alone, the runner answering the
  GET with the frames it captured (F-458).
- **The echo tool** is the observer in `py-py` §3 and §4: scheduled time
  with its jitter, `within` with its delta, an unheld frame by its
  envelope.

The profiles' scenarios on their revisions 1.1:

| Scenarios | zk2py's owner | The owner example |
|---|---|---|
| `health.v1` 0.8, §1 to §8 | 30/30 | not run: the runner drives zk2py's owners |
| `alarms.v1` 0.11, its short-ttl copy (4 s) for §2, §4, §6, §10 | 79/79 | 77/79, then §5's 11/11 after F-460's reader fix |
| `config.v1` 0.9 | 51/51 | 51/51 |
| `freshness.v2` `scenarios-v2.md` §1 to §7 (`beacon.v2` 0.2) | 32/32, and 25/25 with zenctl | 32/32 |

- **No re-put** in any of them: S hears each member put once, then
  heartbeats; a stopped vouch, a closed writer or the clock guard clears
  the bit, and the member goes stale past the horizon.
- **`alarms.v1` §10** establishes `lab/b` by a window whose latest
  heartbeat is in step with T's view, and §2.14 step 5 reads no
  subscription delivery as an earlier instance's.

**Not run:** `link.v2`'s scenarios §1 and F-436's fix, as in round 96;
`alarms.v1`'s §2, §6 and §10 at 900 s (`--full`); a frame resource with
`history`, which zk2py's owner does not publish.

Round 97 found F-455 to F-460, six, none a blocker. See "At core 0.87
and observer 0.21" in `SPEC-FINDINGS.md`.

## Core 0.88 and 0.89, observer 0.22, `freshness.v2` 0.4, `health.v1` 0.9, `alarms.v1` 0.12 and 0.13, `link.v1` 0.14, and `telemetry.v2` 0.1: frames as metric series (round 98)

Since round 97 main took:
- **`telemetry.v2` 0.1** (#983 T1, keyed present by round 97, read now):
  a frame's fields as metric series, §2.16 to §2.21 over v1's §2.1 to
  §2.15 by reference, and its owner (T3, no text change);
- **#978's F5:** the adopters' contracts and `nav.v2` onto
  `freshness.v2`, each a `uses_changed` revision in
  `examples/zk2/.history`; `freshness.v1` 0.5 experimental;
- **#978's F6:** core 0.88, the clock guard narrowed to the state writes
  that carry a Zenoh timestamp; `freshness.v2` 0.4, heartbeats vouching
  with their time all ones; `health.v1` 0.9, the status not held and the
  clock question by the heartbeats' time, with `clock.json`; `alarms.v1`
  0.12;
- **mid-round, f42e9742:** core 0.89, observer 0.22, `link.v1` 0.14 and
  `alarms.v1` 0.13, the answers to round 97's F-455 to F-460. zk2py
  rebased onto them, and each answer holds.

zk2py diffed every keyword sentence of the changed texts between 7597efb8
and f42e9742. **No rule changed beyond what the changelogs state.**

### The classes zk2py claims

Unchanged at the core: the **owner**, **consumer**, **caller** and
**observer** classes, at the full level. It now follows `telemetry.v2`:
an owner (its frames metered) and a tool (R).

### Its walk of `RULES.md` at 0.89

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 75 | 64 | 3 | 8 |
| consumer | 21 | 17 | 0 | 4 |
| caller | 13 | 13 | 0 | 0 |
| observer | 83 | 79 | 2 | 2 |
| static, in every class | 62 | 60 | 0 | 2 |

- **228 rows walked of 228, none without a verdict.** The walk was
  re-keyed first, in its own commit: §4.3's "Ahead" MUST now keyed to "the
  state writes that carry a Zenoh timestamp" (met). 0.89 moved no row.
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's
  two decoding MUSTs.

### What round 98 changed in zk2py

- **The guard** (core 0.88): `Owner._stamped`, a write on §4.3's table or
  on a session that stamps, is refused while the guard holds, re-puts
  included; every other state write goes on. `freshness.v2` heartbeats go
  on vouching, their time all ones, each still minting the members' last
  confirmation from the clock ahead. A covered member whose write the
  guard holds (a candidate's session) has its bit cleared until a put of
  it goes through (F-461's reading).
- **`health.v1` 0.9:** `clock_question` over `heartbeat`,
  `heartbeat_held`, `status_put` and the faults, `time_unknown` among its
  reasons; `heard_from` reads the subscriber's heartbeats and their time
  (`freshness.Subscriber` keeps each body's `time`); the `clock_ahead`
  repeat at the heartbeat's period, 28.5 s.
- **`telemetry_v2`:** §2.16's declaration, §2.18's readings, §2.19 to
  §2.21's pairs (the run as the epoch, the serial window, the time base
  and its basis, a scheduled rate's bound), §5 over many pairs, and the
  owner's `Meter`. `Owner.record_frame` meters a `telemetry.v2` frame
  key: a count below one written in the run is refused before the
  reduction, each count written modulo its width, the counts started
  again under a new run, and an increase past 2^(w−1) named.
  `Owner.raw_frame` is the scenarios' raw step.
- **`telemetry_v2_live.SeriesSubscriber`**, the scenarios' R: one series
  reader per frame key, the bare metric streams by v1 with no presence.
- **W105** reads `telemetry.v2`'s vocabulary, `series` beside v1's four
  keys (F-463).
- **The interop contracts (#978 F5):** `desired/configs.v1` moves onto
  `freshness.v2` (revision 1.1), as the reference's copy moved; its one
  state has no horizon, so it forms no collection. Kept on `freshness.v1`,
  each mirroring a text that keeps it or zk2py's own reading of v1:
  `freshness/beacon.v1` (its scenarios'), `state/zk2py_ground.v1`
  (`state.md` §1's `status`), the three `tasking.v1` and
  `zk2py_tasking2.v1` (`desired.v1`'s scenarios'), `freshness/zk2py_fresh.v1`
  (its re-puts, read by the Rust `consume` example) and `zk2py_probe.v1`
  (zk2py's own, its re-puts checked by the live runner).
- **The mend of round 97's review (PR #1037):** `frame_live`'s callback
  records any put it cannot read, `{"undecodable": why}`; a `within`
  field's `since` checked against the owner's across full records, which
  cannot drift; `alarms_scenarios`' 900 s contract and 905 s waits only by
  `--full`; `except Exception`; the published-review acceptance tied to
  core 0.89 §9.7.
- **`readings`: 860, up from 840.**

**`just py-conformance`: 5113 of 5113**, on main f42e9742: `telemetry`
350 (`v2/` and `conformance/contracts/` read), `health` 120 (`clock.json`
29), `readings` 860.

### Live

`python -m zk2py.telemetry_v2_scenarios`, `scenarios-v2.md` §1 to §7
against the black boxes of main f42e9742, over `meterframe.v1`:

| Section | zk2py owner, zk2py R | owner example, zk2py R | zk2py owner, zenctl |
|---|---|---|---|
| §1 rates, measured and scheduled | 7/7 | 7/7 | 4/4, 5 not judged |
| §2 a restart, a reset | 5/5 | 5/5 | 2/2, 3 not judged |
| §3 a 24-bit wrap | 3/3 | 3/3 | 2/2, 2 not judged |
| §4 a lost frame | 4/4 | 4/4 | 2/2, 1 not judged |
| §5 a 64-bit decrease across a face | 4/4 | 4/4 | 3/3, 2 not judged |
| §6 a rewind under a restored run file | 3/3 | 3/3 | 2/2, 1 not judged |
| §7 one counter, two series | 4/4 | 4/4 | 3/3, 3 not judged |

- **zenctl** (`watch`, and `get` reading the captured frames one reply
  each) shows each record's field states; its telemetry verbs read no
  frame yet (#983 T2), so every step of a series, a pair or a rate is
  "not judged". `zenctl telemetry` reads `meterframe.v1`'s bare counter
  by v1's rules and no frame resource.
- **The owner example's instants** of two records measured 1 s apart
  differ at times by 999 ms; the rates are checked as the formula's over
  the elapsed time read, that time within the conventions' 50 ms.

F6, live:

| Scenario | zk2py's owner | The owner example |
|---|---|---|
| `state.md` §7 step 6 | 3/3 | 3/3 |
| `freshness.v2` `scenarios-v2.md` §2 step 4 | 4/4, and 4/4 with zenctl | 4/4 |
| `health.v1` scenarios §4 steps 1 to 4 | 6/6, [moved clock] 2/2 | 5/5 (`--only 4r`), steps 1 to 3 |
| `alarms.v1` 0.13 §1, §3, §5 | 27/27 | 27/27 |

Each was run again against the black boxes of f42e9742, with
`freshness.v1`'s §2 (5/5) and `frames.md` §1 to §6: 125 of 126 once,
`rust-py` §2's wire wait ending before seq 15 arrived, then 6/6 on two
reruns.

**Not run:** the [moved clock] tier against the owner example (the
runner drives zk2py's session clock); a candidate of a `freshness.v2`
contract with its clock ahead (F-461); `link.v2`'s scenarios §1 and
F-436's fix, as before.

Round 98 found F-461 to F-464, four, none a blocker. See "At core 0.89
and telemetry.v2 0.1" in `SPEC-FINDINGS.md`.

## Core 0.90 and 0.91, observer 0.23, `freshness.v2` 0.5, `telemetry.v2` 0.2, and `link.v2` 0.3 to 0.5: the contract side, frames per face and the far side (round 99)

Since round 98 main took:
- **core 0.90, `freshness.v2` 0.5 and `telemetry.v2` 0.2,** the answers
  to round 98's F-461 to F-464;
- **`link.v2` 0.3** (#988 L3): the contract side (§2.17's packed
  requirement) and frames per face (§2.7, §2.8, §2.12, §2.13, §2.16,
  §2.18), with `v2/exposure.json`, new `lints.json` and `plan.json` cases
  and `conformance/contracts/probe.v1.toml`;
- **core 0.91, observer 0.23 and `link.v2` 0.4** (#988 L4): the far side,
  core §8.1's reply timeout and §8.5's declaring again, observer §7.5, and
  `constrained.md` §8 and §9;
- **#983's T2** (no rule): `zenctl telemetry` reads frames;
- **mid-round, 2ed6d968 (#1048):** `modem.v3` 3.2 on nine `telemetry.v2`
  frame resources, `gendev.v1` 1.1's `frames/sample`, zenoh-modem's faces
  on `link.v2` (0.5, five new `plan.json` cases), `telemetry.v2` draft, and
  editorial `link.v1` 0.15 and `telemetry.v1` 0.8. zk2py rebased onto it.

zk2py diffed every keyword sentence of the changed texts between f42e9742,
bdb12e5e and 2ed6d968. **No rule changed beyond what the changelogs
state.**

### The classes zk2py claims

Unchanged at the core: the **owner**, **consumer**, **caller** and
**observer** classes, at the full level. It now follows `link.v2` as a
tool (0.3's plan) and as a far side's caller, consumer and tool (0.4).

### Its walk of `RULES.md` at 0.91

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 75 | 64 | 3 | 8 |
| consumer | 23 | 19 | 0 | 4 |
| caller | 14 | 14 | 0 | 0 |
| observer | 86 | 82 | 2 | 2 |
| static, in every class | 62 | 60 | 0 | 2 |

- **234 rows walked of 234, none without a verdict.** The walk was
  re-keyed first, in its own commit: core §8.1's SHOULD under the consumer,
  the caller and the observer, §8.5's under the consumer and the observer,
  and observer §7.5's, each not met there, then met with the far side.
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's
  two decoding MUSTs.

### What round 99 changed in zk2py

- **`link_v2`, text 0.3:** a face's `frames` and `horizon_s`, each entry
  checked alone; `FaceReading`, handed to `link.plan_face`, denies an
  unlisted frame and a timed stream, reads a listed frame's cadence as its
  `every_s`, and warns `not_packed`, `timed_downsampled`,
  `horizon_under_face`, `block_over_batch` and `keep_alive_not_suppressed`;
  across the faces `frame_unlisted`; and the refusals `packed_required`,
  `frame_unknown`, `keyframe_unreachable` and, given the far side's
  catalog, `catalog_missing`. A resource's type kind, congestion and
  longest put are read from each revision (F-469's reading), and a loaded
  revision carries its fingerprint. An implied `freshness.v2` takes its
  stream deny alone, as the fixtures do (F-466). `v2/exposure.json` is read.
- **`far_side`** (core §8.1, §8.5, observer §7.5, `link.v2` §2.20,
  §2.21): `FarSide`, one face of a `link.v2` policy; `live.call` with no
  timeout of its own waits the operation's `timeout_ms`, else 10 s, across
  a face at least its reply timeout, one it sets taken at its word;
  `live.get_state` and a seed floored at the face's; `Redeclaring`
  undeclares and declares anew a subscription silent for two horizons, its
  transport up, through `Owner.subscribe_role(across=, horizon_s=)` and
  `echo_tool.watch`; `echo_tool --face-policy F --face NAME`, its session on
  the face's `[transport]`.
- **The answers:** `keeps_timestamp` stamps a `telemetry.v2` state metric
  (F-462); `SeriesReader.under` reads a frame under the revision it names
  (F-464); the re-put path clears a held member's bit (F-461).
- **`adopter_frames`** (the coordinator's addition, #983's last "Done
  when"): `modem.v3` 3.2's nine frame resources and `gendev.v1`'s
  `frames/sample`, written by zk2py's `Framer`, read back envelope first
  and through `SeriesReader`. `gendev.v1.toml` is the one `docs/` file the
  coordinator opened, read as contract TOML; zk2py's loader reads its two
  schema files beside it.
- **The mend of round 98's review (PR #1041):** the re-put path; R's
  exception recorded as `{"error": why}`, apart from an undecodable
  payload, a bare sample conformed outside the lock; the meter gathers,
  and accepts a frame's counts once its put goes through, a failed put
  giving its seqs back; `alarms_scenarios`' `--full`; `clock_question`'s
  docstring in its checks' order.
- **`readings`: 872, up from 860.**

**`just py-conformance`: 5173 of 5173**, on main 2ed6d968: `link` 273
(`v2/` lints 40, plan 19, exposure 18, transport 16), `telemetry` 351
(`continuity.json` 14), `readings` 872.

### Live

| Run | Pairing | Result |
|---|---|---|
| `far_side_live`: `constrained.md` §8 steps 1–2, §9 steps 1–4, in process, a proxy dropping a batch | zk2py's owner, caller, consumer and tool | 16/16 |
| `freshness.v2` `scenarios-v2.md` §2, step 6 new | zk2py owner, zk2py readers | 13/13 (step 6: 5/5) |
| | owner example, zk2py readers | 10/13 (step 6: 2/5, F-467) |
| | zk2py owner, `zenctl watch` | 10/10 (step 6: 2/2) |
| `telemetry.v2` `scenarios-v2.md` §1–§7 | zk2py owner, zenctl (`watch`, `get`, `telemetry`) | 35/35, every step judged |
| | zk2py owner, zk2py R; owner example, zk2py R | 30/30 each |
| `constrained.md` §7 (regression) | zk2py | every check |

- **`constrained.md` §8:** the call setting no timeout drew the router's
  `Timeout` at 1.00 s and no value; across the face, the value at 1.50 s.
- **§9:** a dropped 13-byte `DeclareSubscriber` healed once at 2.02 s, for
  the consumer and the tool; with `arq = true` nothing in 3.5 s; a dropped
  49-byte batch holding the key's `DeclareKeyExpr` stayed silent, declared
  again once at 2.02 s. The proxy reads zenoh's stream framing, and skips
  a keep-alive's 1-byte batch when told to drop "the next batch".
- **`zenctl telemetry`** reads every frame pair as R does; it reads
  presence where R reads none, so §7's bare pair is established by
  presence where R assumes it. Its frame pairs say `presence none`.
- **Frames' sizes** (`adopter_frames`), link.v1 §2.8's largest frame:
  `frames/link` 22 B, `frames/message` 213 B, `frames/sample` 97 B, as the
  Rust side measured; `frames/air` 49, `bearer` 32, `if` 58, `layer` 106,
  `radio` 21, `sched` 32, `sdu` 51. A delta of every field is 1–3 B more
  where `keyframe_every` is above 1 (F-470).

**Not run:** `link.v2`'s scenarios §1 and §2 on routers (§2's plan,
expected 1 and 2, is a reading); core §8.5's MAY, a session opened again;
`constrained.md` §9 on a real radio; F-436's fix, as before.

Round 99 found F-465 to F-470, six, none a blocker. See "At core 0.91,
observer 0.23 and link.v2 0.4" in `SPEC-FINDINGS.md`.

## Core 0.92 and 0.93, `link.v1` 0.16 and `link.v2` 0.6: a call's forms, and round 99 answered (round 100)

Since round 99 main took:
- **core 0.92** (#985 C4a): a call's forms defined, the request, form 2's
  value and form 3's error, with the error's byte, in §5.1, §5.2, §7.1,
  §7.5, §9.2's E044, narrowed, and §12. No rule adopts them: every call is
  still answered in §5.2's envelope. Four vectors move from `form` to
  `sample_kind`, and `envelope/vectors.json` gains 91 cases. New are
  `errors/forms.json` and `contracts/ok-packed-operation`;
- **core 0.93, `link.v1` 0.16 and `link.v2` 0.6:** the answers to round
  99's F-465 to F-470, with new `plan.json` and `v2/plan.json` cases.

zk2py diffed every keyword sentence of the changed texts between
2ed6d968 and 037fce56. **No rule changed beyond what the changelogs
state.**

### The classes zk2py claims

Unchanged: the **owner**, **consumer**, **caller** and **observer**
classes, at the full level, and the static rules, which now hold a
call's forms (core §12).

### Its walk of `RULES.md` at 0.93

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 75 | 64 | 3 | 8 |
| consumer | 23 | 19 | 0 | 4 |
| caller | 14 | 14 | 0 | 0 |
| observer | 86 | 82 | 2 | 2 |
| static, in every class | 64 | 62 | 0 | 2 |

- **236 rows walked of 236, none without a verdict.** The walk was
  re-keyed first, in the commit that brings `just py-conformance` level.
  §7.1's E044 is narrowed. §7.5's two writer rules are new, both met.
  Its parse and write-back rules now read "an envelope or a form".
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's
  two decoding MUSTs.

### What round 100 changed in zk2py

- **`zk2c`, core 0.92:**
  - forms 2 and 3 are defined; 4 and 7 stay reserved;
  - `parse` reads a `query`, a `reply` and a `reply_err` in §7.5's order:
    `request_run` after `tag`, `short` counting an error's byte, then on
    that byte `cause`, `detail`, `extension`, a message's `short`, `utf8`
    and `trailing`;
  - a request's width, run and seq are None, and an error's byte, message
    and unread detail are an `ErrorForm`;
  - `write_back` writes every accepted form again;
  - `request`, `value` and `write_error` write the forms. `write_error`
    cuts a message to whole code points within 255 bytes, and refuses
    `cause`, then `detail`, as the parse would;
  - `read_reply_err` reads a reply error by its Encoding first.
- **No zk2py owner answers in a form, and no caller sends one** (core
  §5.1, "The forms"). zk2py's caller still reads a reply error under any
  but §5.2's three encodings as the transport's, a router's `Timeout`
  under `zenoh/bytes` too.
- **The errors family** reads `forms.json`: its codes and causes in the
  byte's order, all 256 bytes, 22 writes each parsed back as cut, and 17
  reads.
- **The linter** takes the narrowed E044: an attachment and an explicit
  stream's type alone. `ok-packed-operation`'s fingerprint is the
  reference's.
- **The answers.** A refused deployment lists its errors alone (F-468).
  Each revision is judged alone, a longest put weighed in its `block`
  revisions (F-469). The largest frame holds `records` −
  ⌊`records`/`keyframe_every`⌋ delta records of every field, the largest
  any revision writes (F-470). The runner reads one interface id given by
  contract and inline as two revisions of it.
- **The mend of round 99's review (PR #1049):**
  - `Redeclaring.tick()`, driven directly with no real sleep;
  - the stop race closed under the lock;
  - `history=` noted as zenoh-python 1.10.1's keyword-only argument;
  - `adopter_frames` pairs records by seq;
  - the nits.
- **`forms_live`** (new): Appendix B's router `Timeout` measured, and
  both owners on a packed operation (F-471).
- **`readings`: 883, up from 872.**

**`just py-conformance`: 5639 of 5639**, on main 037fce56. Of them,
`envelope` 275 (147 cases, 20 headers, each accepted one written back),
`errors` 361 (`forms.json` 316), `contracts` 144, `link` 277 and
`readings` 883.

### Live

| Run | Pairing | Result |
|---|---|---|
| `freshness.v2` `scenarios-v2.md` §2, step 6 checking the candidate's stamps | zk2py owner, zk2py readers | 14/14 (step 6: 6/6) |
| | owner example, zk2py readers | 14/14 (step 6: 6/6, F-467) |
| | zk2py owner, `zenctl watch` | 10/10 (step 6: 2/2) |
| `forms_live`: a held query's reply error, both owners on `ok-packed-operation` | zk2py, the owner example | 6/6 |
| `far_side_live` (regression after the mend) | zk2py's owner, caller, consumer and tool | 16/16 |

- **F-467:** the owner example as the candidate, 2 s ahead: its guard
  holds the second put, and every put S hears carries its own zid.
  `--no-hlc` beside `--standby` is refused at start.
- **Frames' sizes** (`adopter_frames`, `link.v1` 0.16 §2.8): `frames/link`
  22 B, `frames/message` 213 B, `sdu` 52, `sched` 33, `air` 50, `layer`
  109, `if` 60, `radio` 22, `bearer` 33, as the modem README states;
  `gendev.v1`'s `frames/sample` 100 B.
- **A router's `Timeout`:** one reply error per held query, under
  `zenoh/bytes` at 1.00 s. At 2.50 s it came under `zenoh/bytes` in one
  run and under the querier's `zenoh/string` in another.

**Not run:** the C4b reading of both wires, which is the reference's;
`link.v2`'s scenarios on routers; core §8.5's MAY; and F-436's fix, as
before.

Round 100 found F-471, one gap, not a blocker. See "At core 0.93 (#985
C4a, round 99's answers)" in `SPEC-FINDINGS.md`.

## Core 0.94 and `observer.md` 0.24: the flip, every call in the value and error forms (round 101)

Since round 100 main took:
- **core 0.94** (#985 C4c, the flip): a caller writes its request in
  §7.5's request form, and an owner its values and its errors in the
  value and error forms, each under `zenoh/bytes` with its as-of mark,
  whatever its types' kind. An answer sent again is its first bytes, mark
  included. A `reply_err` under `zenoh/bytes` is read by §7.5's parse, and
  every other one is the transport's. A reader MAY still read the bare
  wire, any `Encoding` but `zenoh/bytes`. New are `envelope/calls.json`
  and `operations.md` §9;
- **observer 0.24:** §7.3, "A call's forms first";
- **config.v2 0.1** (#985 G1), present and not read: #985's later round
  reads it after G2 and G3, so the config family names `v2/` and
  `contracts/` as present and not read, and files nothing on them;
- #1044, #1047 and the zenoh-modem migration guide, with no rule change.

zk2py diffed every keyword sentence of the changed texts between
037fce56 and fd50c100. **No rule changed beyond what the changelogs
state.** `RULES.md` lists one rule under the caller alone that binds a
tool too (F-472).

### The classes zk2py claims

Unchanged: the **owner**, **consumer**, **caller** and **observer**
classes, at the full level, and the static rules. config.v2 is listed as
not followed.

### Its walk of `RULES.md` at 0.94

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 77 | 66 | 3 | 8 |
| consumer | 23 | 19 | 0 | 4 |
| caller | 18 | 18 | 0 | 0 |
| observer | 90 | 86 | 2 | 2 |
| static, in every class | 64 | 62 | 0 | 2 |

- **245 rows walked of 245, none without a verdict.** The walk was
  re-keyed first, in the commit that brings `just py-conformance` level,
  with the new owner and caller rules "not met". The flip's commit makes
  them met.
- §5.2's `Encoding` MUST is gone. The observer's verdict on "Decoding"
  stands beyond the index (F-472).
- **Not met:** unchanged, the owner's three SHOULDs and observer §7.3's
  two decoding MUSTs.

### What round 101 changed in zk2py

- **`calls`** (new): an operation's forms by its types.
  - Each CH's tag is a packed type's, else 0.
  - A record is encoded and read as its type says: a packed one through
    §7.6 under CH's tag, a JSON Schema one as JSON or CBOR on the
    operation's `encoding`, a protobuf or raw one as its bytes.
  - An error's detail its type refuses keeps the error the owner's.
  - `zk2c.check_error` holds §7.5's writer checks, and `cbor` gains a
    writer.
- **The owner** (`owner.OpCall`, `owner._refuse`):
  - every value and summary is the value form, and every refusal the
    error form, under `zenoh/bytes`, with `Owner.mark`'s as-of mark: the
    run it writes under, and the last state-and-event seq it minted, or 0;
  - a request is read by §7.5's parse, and its record as the `request`
    type, a packed one under CH's tag. Either refusal is `invalid_request`,
    after the three refusals before any handler;
  - a request under another `Encoding` is the bare wire, its payload the
    record;
  - `refuse(detail=)` takes a value of the `error` type, encoded as it
    says. A detail it cannot encode raises before anything is sent, so the
    call ends `internal`;
  - `OpCall.resend` sends an answer's first bytes again.
- **config.v1's owner** keeps each first answer's bytes (`Kept.wire`) and
  resends them on a retried `set` (§2.14). A dropped reply is made and
  kept, never sent. Its `ConfigError` detail is that type's JSON bytes.
- **The caller** (`live.call`):
  - it writes `CH · record` under `zenoh/bytes`, the tag the request
    type's;
  - it reads each reply by §7.5's parse (`live.read_reply`): a refused one
    is `refused_value`, an answer whose bytes are never handed on;
  - it reads each `reply_err` by §5.2 (`live.read_reply_error`): a form
    the parse accepts is the owner's error, every other one the
    transport's;
  - it takes §5.1's MAY: under the envelope's three encodings it reads the
    bare wire's envelope, and one its decoding refuses is the transport's
    (F-473). `read_bare=False` is a caller that does not;
  - `attempts` retries an idempotent concrete call after silence alone
    (O4).
- **The tool:** `live.show_reply` shows a reply as observer §7.3 says. A
  value is decoded as its type where the operation is held. A refused
  value is named. An error is shown by its code, cause and message, its
  detail decoded by the `error` type. The transport's is shown with its
  `Encoding` and its bytes. The as-of mark is shown as a run and a seq.
- **The envelope family** reads `calls.json` whole: 3 types, 3 forms, 21
  writes, 18 reads, and each accepted read written back.
- **`forms_live`** (rewritten): `operations.md` §9 in three pairings, over
  `interop/operations/tc.v1.toml` (new), beside `ok-packed-operation`.
- **`readings`: 896, up from 883.** Round 100's readings of 0.92's "no rule
  adopts them" are re-keyed to their absence. Round 101's readings state
  0.94's rules, and F-472 to F-476 as zk2py reads them.

**`just py-conformance`: 5711 of 5711**, on main b483902e. Of them,
`envelope` 332 (`calls.json` 57), `errors` 361, `config` 332 (`v2/` and
`contracts/` keyed, not read) and `readings` 896.

### Live

`operations.md` §9, step by step (`forms_live`):

| Step | zk2py caller, zk2py owner | zk2py caller, the owner example | `zenctl call`, zk2py owner |
|---|---|---|---|
| 1, each kind of type | 28/28 | 16/16 | 20/22 (F-478) |
| 2, packed types | 5/5 | 2/2 | 1/5 (F-477) |
| 3, the wrong tag | 2/2 | 1/1 | 2/2 |
| 4, the as-of mark | 2/2 | 2/2 | 2/2 |
| 5, errors that are no owner's | 4/4 | as py-py's caller | 4/4 |
| 6, a timeout behind a router | 1/1 | as py-py's caller | 1/1 |
| 7, the bare wire | 2/2 | as py-py's caller | 1/1 |

- **The owner example** answers with its default handlers. A raw
  operation echoes, in the value form. Every other is refused `app` with
  no detail. So its step 1 and 2 checks are its CHs and its refusals:
  `calibrate` 0x72 under Change's tag, `scan` and `reset` 0x70. It reads
  zk2py's request forms, and step 3's tag-0 Sensor is `invalid_request`.
  Its marks follow its puts, and a new instance's reply carries seq 0.
- **Steps 5 to 7** play no owner, so they test the caller. zk2py's runs in
  `py-py`, and is the same in `py-rust`.
- **`zenctl call`** reads every reply error as §5.2 says. It reads the
  bare wire, step 7's envelope being `busy`, exit 1. Two bugs:
  - **F-477:** it refuses a packed request type, nothing sent: "which no
    caller writes yet";
  - **F-478:** it reads a CBOR operation's value and detail as JSON, and
    shows them undecodable.
- **F-476:** in each pairing, a second instance's reply before its first
  put, that put and a reply after it all carry seq 0.
- **A router's `Timeout`:** zenoh-python's caller drew its own under
  `zenoh/string` 6 times of 6, at 1 s and 2 s. `zenctl call` drew the
  router's under `zenoh/bytes` 4 times in 6. Both are silence.

config.v1's scenarios through the forms:

| Run | Result |
|---|---|
| `config_scenarios`, zk2py's owner | 52/52 |
| `config_scenarios`, the owner example | 52/52 |

- §10 step 2 now compares the bytes. With zk2py's owner, the retried
  `set`'s reply is the dropped reply's bytes, its mark included. With the
  owner example, it is the first reply's bytes, which C set aside.

The regression, after the flip:
- `far_side_live` 16/16, and `observer_scenarios` §5 and §8 step 6 8/8.
- `live_interop`, whole on the black boxes, 442 of 463.
  - **Two failures were the flip's:** operations.md §1's controls sent a
    bare `get`, which zk2py's owner now refuses `invalid_request`. They now
    send the request form, and `--only o1` passes whole.
  - **One is the reference's:** F-479, the `consume` example printing a raw
    value with its form.
  - **18 involve no call:**
    - 10 on the owner example's descriptors, tokens, `health.v1` re-puts
      and clock steps, which list or expect no `freshness.v2`;
    - `s7`'s 7, which look for an older black box's path;
    - `consume`'s stop on `health.v1`, which now exits 1 on `@op/none`.

    They are not re-keyed, and not investigated, this round.

**Not run:** the C4b reading of both wires, which is the reference's;
`link.v2`'s scenarios on routers; core §8.5's MAY; and F-436's fix, as
before.

Round 101 found F-472 to F-479: one contradiction, one ambiguity, three
gaps and three bugs in the reference. None is a blocker. F-471's answer holds.
See "At core 0.94 and observer 0.24 (#985 C4c)" in `SPEC-FINDINGS.md`.

## Core 0.95 and 0.96, `observer.md` 0.25 and 0.26, `config.v2` 0.1, `link.v2` 0.7 and `modem.v3` 3.3: round 101 answered, a configuration in its contract, and a first hop (round 102)

Since round 101 main took:
- **core 0.95 and observer 0.25:** F-472 to F-476 answered on the texts,
  and F-477 to F-479 fixed in the reference;
- **config.v2 0.1** (#985 G1), now **draft**, used by zenoh-modem's
  `sx1262_config.v1` (#985 G4), with its owner (G2) and tools (G3) in the
  reference;
- **link.v2** draft (a status entry, text 0.6);
- #1055 and #1064 in the reference: a config.v1 or config.v2 owner's
  re-mint restarts its configuration;
- **core 0.96, observer 0.26, link.v2 0.7 and config.v1 0.10** (#982 H1,
  main 0676db19): an owner on a first hop holds its instance token alone,
  a reader finds a provider by its instance token, and a device face
  carves its devices' instance keys from `zk`. With H2 in the reference,
  the owner example takes `--first-hop`.

Round 102 also carries PR #1059 (#991, a680b029): **`modem.v3` 3.3**,
three frame resources over one record of all 73 readings, catalog
e3a529fe, and `link.v1` 0.17, `link.v2` 0.8 and `telemetry.v2` 0.3, each
editorial.

zk2py diffed `spec/` between b483902e and ee5fb697, between ee5fb697 and
0676db19, and between 0676db19 and a680b029. **No rule changed beyond
what the changelogs state.** One sentence of `scenarios-v2.md`'s intro,
naming zenctl's tests, has no entry (F-491). #1059 changed no line with a
keyword.

### The classes zk2py claims

Unchanged for the core: the **owner**, **consumer**, **caller** and
**observer** classes, at the full level, and the static rules. Of the
profiles, **config.v2** is claimed now: its owner (a simulated device),
its caller, its tool (§5) and its linter (§2.2). zk2py's `link.v2`
generator takes the device face.

### Its walk of `RULES.md` at 0.96

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 82 | 69 | 3 | 10 |
| consumer | 23 | 19 | 0 | 4 |
| caller | 18 | 18 | 0 | 0 |
| observer | 93 | 89 | 2 | 2 |
| static, in every class | 64 | 62 | 0 | 2 |

- **254 rows walked of 254, none without a verdict.**
  - At 0.95, 246. The new row, §5.2's Decoding MUST under the observer
    (F-472's index), walks the verdict round 101 held beyond the index.
  - 0.96 adds eight. The owner's three are met, by zk2py's owner on a
    first hop. The observer's three are met, by its reader and its tool.
  - The other two are n/a: a deployment's SHOULD and a tool's MUST NOT
    the index lists under the owner too (F-492).
- The walk was re-keyed first, in the commit that brings `just
  py-conformance` level: round 101's readings of F-472 to F-476 now read
  their answers.

### What round 102 changed in zk2py

- **`config_v2`** (new), from `v2.md` and the fixtures alone:
  - `read_groups`: `config.groups` read into groups and markers, or the
    reasons in the text's order; `derive` and `layouts`: `Record`, each
    `Set<T>`, `Control` and `Error`, their tags, layout hashes and bit
    bounds; `lint`: the contract's findings, `resolved` adapting a loaded
    contract;
  - the record: an owner's state written as `Record`'s JSON reading and
    its bytes, and read back with §2.3's refusals after core §7.6's
    (`group`, `pending`, `last`, `desired`, `parameter`), two revisions
    reading each other;
  - `check_request`: persist without a layer, the parse, then the rows;
    `check_desired`; `deadline_ms` and `round_up` (§2.8);
  - `Core`: the owner as a state machine, its puts, replies in the value
    and error forms under their marks, events and acts; tokens of 48
    bits, a new run when the seq or the counter is spent; the first
    answer replayed byte for byte; redaction;
  - the session half: `LiveCore` on zk2py's `Owner` (its run, its one
    count of seqs, each reply sent as it is made, before the event);
    `ConfigV2Owner` (the record before the tokens, deadlines on a timer,
    random counters, a re-mint that restarts the configuration); `Caller`,
    which holds the current record by seq within a run; `Recorder`;
    `tool_answers`, §5's six questions;
  - `sx1262_round_trip`: `config.v2-binding.md` §15 through zk2py's own
    owner and codec.
- **The config family** reads `v2/` and `contracts/` whole: 593 of 593,
  up from 332. **The examples family** lints every contract that uses
  config.v2 (§2.2: "An implementation that lints a contract MUST report …
  a finding as an error"): `sx1262_config.v1` is clean.
- **`live.read_reply`**, for a caller that takes no MAY: a reply under
  another `Encoding` is malformed (core 0.95 §5.1).
- **`forms_live`** step 7: `ping` answered by an envelope with no
  `message`, the transport's to every caller (core 0.95 §5.2).
- **`readings`: 912, up from 896.** Round 101's F-472 to F-476 read as
  answered; core 0.95's, observer 0.25's, config.v2's and link.v2's
  entries; config.v2's fixture counts; the SX1262's sizes; F-480 to F-491
  as zk2py reads them.

**`just py-conformance`: 5993 of 5993**, on main ee5fb697. Of them,
`config` 593 and `readings` 912.

At core 0.96 (#982 H1, main 0676db19):
- **`link` and `link_v2`**, the device face from the text alone: `attach
  = "device"` and `devices` (`bad_value`, `device_listed`), its devices
  read as listed services, the carve of `zk` into three denies,
  `presence_required` and `epoch_on_device`, no downsampling of a
  device's stream on its own face, and `zk_not_denied` over the carve.
  `v2/plan.json`'s 22 to 26 and `v2/lints.json`'s 40 to 57 pass, the link
  family 300 of 300.
- **`owner.Owner(first_hop=True)`:** every interface tokenless, its
  instance token alone, no contract queryable, and `archive.v1` and a
  governed interface refused before anything is declared.
- **`first_hop`** (new): the tool's report of a first-hop owner, its link
  closed and never down. **`first_hop_live`** (new): `constrained.md` §10.
- **`classes`:** 0.96's eight rows, two of them n/a (F-492).
- **`readings`: 917**, core 0.96's, observer 0.26's, link.v2 0.7's and
  config.v1 0.10's entries and rules, and F-492.

**`just py-conformance`: 6021 of 6021** on main 0676db19.

Carrying #1059 (`modem.v3` 3.3, #991):
- **#983 T4's rows** read 3.3: 13 frame resources, 3.2's nine, the three
  new, and `gendev.v1`'s `frames/sample`. Each is written by zk2py's
  `Framer`, read back and written again byte for byte.
- **F-470's rows** read 3.3 too: `frames/fast` and `frames/slow` are
  (207, 216) as (full, largest) by `link.v1` 0.16 §2.8, and `frames/paid`
  is 207 B. zenoh-modem's README states each of the twelve sizes zk2py
  measures.
- **`readings`: 927**, 3.3 read as an adopter's contract:
  - the three entries;
  - the contract's counts, and its review against 3.2;
  - its frames, their series, and `Readings`' bits and sizes;
  - the packed states;
  - catalog e3a529fe;
  - the five faces planned as `v2/plan.json`'s 14 to 18;
  - F-493 and F-494.

**`just py-conformance`: 6031 of 6031** on the rebased branch (a680b029
and round 102). Of them, `config` 593, `link` 300 and `readings` 927.

### Live

`scenarios-v2.md`, section by section (`config_v2_scenarios`):

| Section | zk2py's owner and caller | the owner example, zk2py's caller | zk2py's owner, `zenctl config` |
|---|---|---|---|
| §1 a hot set, a dry run | 5/5 | 5/5 | 5/5 |
| §2 a reach set, confirmed | 4/4 | 4/4 | 4/4 |
| §3 rolled back at its deadline | 3/3 | 3/3 | 3/3 |
| §4 an undo that fails | 2/2 | 2/2 | 2/2 |
| §5 a join, a second writer | 4/4 | 4/4 | 4/4 |
| §6 an extend | 3/3 | 3/3 | 3/3 |
| §7 tokens, a restart | 5/5 | 5/5 | 5/5 |
| §8 persist, granted and not | 4/4 | 4/4 | 4/4 |
| §9 outside the allowlist | 3/3 | 3/3 | 3/3 |
| §10 an idempotent retry | 4/4 | 4/4 | 4/4 |
| §11 sensitive values | 4/4 | 4/4 | 3/4 (F-485) |
| §12 desired state refused | 3/3 | 2/3 (F-482) | 3/3 |
| §13 across a constrained face | 4/4 | 4/4 | 4/4 |
| §14 a tool's answers | 6/6 | 6/6 | 6/6 |
| a re-mint (config.v1 §2.12) | 4/4 | 4/4 | not run |

- **zenctl config** is C with `set --no-precheck` where the owner's own
  answer is the observable, a sensitive value from a file, and T with
  `get --clocks-synced` (`--for` over §14 step 3's deadline, to hear the
  event). Its §13 runs `--across-face --iface rfcfg.v1`. Its precheck
  refuses `set/modulation` before sending (§9 step 3).
- **§10** drops step 1's reply at zk2py's owner; the owner example cannot,
  so C sets its first reply aside. The 300 s are the owner's clock moved.
- **§13's face** is a deny list under `allow`: an allow list under
  `deny` lost every call across it, the GET alone answered.
- **The re-mint:** both owners restart the configuration; the reference
  also ends the pending change, `rolled-back`, under its old run
  (F-481). S hears the new record and the new instance token in either
  order within a few ms, from either owner, so the check is T's GET at
  the token.

zenoh-modem's SX1262 (`sx1262_round_trip`), through zk2py's own owner:

| Step | Request | Put and reply |
|---|---|---|
| 0, start | — | 30 B (a 24 B record) |
| 1, `set/duty` | 5 B | 36 B, seq 1; the event after |
| 2, `set/radio` | 16 B | 50 B, seq 3 |
| 3, `confirm` | 8 B | `busy`, 7 B |
| 4, the apply | — | 46 B, seq 4 |
| 5, `confirm` | 8 B | 36 B, seq 5; the event after |

The binding's §15 gives 30, 36, 50, 46 and 36 B, and its four payloads
are zk2py's byte for byte. A retry of step 2 is its first 50 B.

`forms_live`: 105 of 105, `zenctl call` 38 of 38 (F-477 and F-478 fixed).

`constrained.md` §10 (`first_hop_live`), on main 0676db19's black boxes:

| Device | Steps 1 to 6 |
|---|---|
| the owner example's `--first-hop` | 11/11 |
| zk2py's `Owner(first_hop=True)` | 11/11 |

- **Each device is R1's client through a proxy** that keeps the bytes
  toward the router. The instance key alone crosses at each transport.
  The descriptor lists every interface `"token": false`, and the bundle
  comes from a near holder.
- **While the link is closed,** a tool that knows the face reports it
  closed, never down. A new run's descriptor is heard.
- **At its defaults** a device sends its interface tokens and contract
  keys. A governed interface refuses the start.
- **The block's halves of steps 4 and 5 are not run.** The owner example
  takes no credentials, so no block can tell its session from the near
  side's.
- **Step 2 now waits for R1 to drop the token,** up to 10 s, where it
  slept 2 s. On the rebased branch R1 dropped it after 2.6 s to 2.7 s, so
  both devices had failed the step at 2 s. 22 of 22 twice since.

**`live_interop`, whole on main 0676db19's black boxes, on the rebased
branch: 490 of 490**, no XFAIL. That is up from the mend's 484 of 490 on
ee5fb697, whose six were T's presence read in the health run,
intermittent.

**Not run:**
- `link.v2`'s scenarios on routers, the device face's carve among them;
- §10's block halves (above), and F-436's fix, as before;
- the README's rf0 sizes, 66 B full and 39 B delta for its 18 readings.
  Which 18 they are is zenoh-modem's code, which zk2py does not read.

Round 102 found F-480 to F-494:
- two contradictions and three ambiguities;
- one gap and one bug in the reference;
- six minor errors;
- an index defect and a changelog gap.

None is a blocker. F-472 to F-479 are resolved. See "At core 0.96,
config.v2 0.1 and modem.v3 3.3 (#985, #982 H1, #991)" in
`SPEC-FINDINGS.md`.

## Core 0.97 and 0.98, `config.v1` 0.11, `config.v2` 0.2 and `telemetry.v2` 0.4: form 7, a collection's snapshot, and round 102 answered (round 103)

Since round 102 main took:
- **core 0.97** (#977 E1): §7.5's form 7, a collection's snapshot, in
  pages. No rule adopts it yet. Its fixtures: `envelope/vectors.json` (12
  new cases, 7 changed), `carries.json` (12 new reads) and the new
  `snapshot/bodies.json`;
- **core 0.98:** the rule index reads a subject's own role (F-492). No rule
  changed, and the owner class counts 76;
- **#1075:** config.v1 0.11, config.v2 0.2 and telemetry.v2 0.4, which
  answer F-480 to F-494. F-485 was fixed in the reference (`zenctl
  config`).

zk2py diffed `spec/` and `examples/zk2/` between cc10c662 and 2688123d.
**No rule changed beyond what the changelogs state.** The keywords added
are §7.5's four, the snapshot's. F-494's fix in zenoh-modem's README and
`MIGRATION.md` has no entry, and examples carry none.

### The classes zk2py claims

Unchanged: the **owner**, **consumer**, **caller** and **observer**
classes, at the full level, and the static rules. The snapshot's four
rules are static: zk2py reads, writes and cuts pages, and no zk2py owner
writes one, as no rule asks yet.

### Its walk of `RULES.md` at 0.98

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 80 | 69 | 3 | 8 |
| consumer | 23 | 19 | 0 | 4 |
| caller | 18 | 18 | 0 | 0 |
| observer | 93 | 89 | 2 | 2 |
| static, in every class | 68 | 66 | 0 | 2 |

- **256 rows walked of 256, none without a verdict.**
  - The static rules gain §7.5's four, each met by `snapshot`.
  - The owner loses round 102's two n/a's, which core 0.98's index no
    longer lists under the owner (F-492).
  - The heartbeat's reading is keyed to "in this order": the snapshot's
    reading shares its first words.
- The walk was re-keyed first, in the commit that brings `just
  py-conformance` level. Round 102's readings of F-480 to F-494 now read
  their answers.

### What round 103 changed in zk2py

- **`zk2c`:** form 7 is defined. A put carries a snapshot, and `key_kind`
  refuses one on a key that is no state's, before `short`. Form 4 is the
  one reserved, so the readings' refused envelope is form 4's CH.
- **`snapshot`** (new), from §7.5 and the fixtures' descriptions alone:
  - `read_page` and `read_body`: the envelope, then the body, refusing in
    the reading's order;
  - `write_page`: every field checked in that order, with the reading's
    token, so no page it writes is refused;
  - `write_back`, `cut` (the paging rule, each candidate page written and
    measured) and `complete`;
  - `cut_by_arithmetic`, a second reading, which sums §7.5's sizes and
    writes no page;
  - `cross_check`, the two readings on random snapshots.
- **The snapshot family** (new) reads `snapshot/` whole: 269 of 269. Each
  page written reads back and writes back. Each set of pages reads back,
  complete, to its snapshot's entries, and is cut by arithmetic too. **The
  envelope family:** 367 of 367.
- **`echo_tool`:** a snapshot on a state key is shown by its page, read
  without the contract, each member's key the reply's key and its id. Its
  body is never decoded as the resource's type, nor sniffed.
- **`config_v2`** and **`config`**, round 102's answers:
  - §2.18 (F-482): each converging change is put, then its event; then the
    report in a put of its own.
  - §2.12 (F-481): a re-mint ends a pending change first in the old
    instance, as `cancel` does: the record is put, then its event. Then
    the device returns to its startup values, then the new instance. A
    runtime parameter with no known startup value, an act in flight and a
    device that will not return each refuse the re-mint.
- **`config_v2_scenarios`** reads 0.2: §6's event at the record's
  deadline (F-490), §12's record, event, then report (F-482), and the
  re-mint's ended change checked (F-481).
- **The mend** (review-bot on #1069):
  - `blackbox.BLACKBOX`, one constant for the ten runners and
    `health_scenarios`;
  - `packed.field_of` public, so `config_v2`'s lint has no private
    access;
  - §8's dead `ds = […]` gone;
  - `_config_v2`'s unknown-fixture gate kept.
- **`readings`: 937**, up from 927:
  - round 102's F-480 to F-494 read as answered;
  - core 0.97's and 0.98's entries, their counts and rules;
  - §7.5's sizes by zk2py's writer;
  - 300 random snapshots cut both ways;
  - the echo tool on a snapshot;
  - F-495 to F-497.

**`just py-conformance`: 6346 of 6346** on main 2688123d. Of them,
`envelope` 367, `snapshot` 269 and `readings` 937.

`snapshot.cross_check(20000)`: 20,000 random snapshots, 793,039 pages,
both readings agree on every page. 3,933 of the snapshots pass 64 pages.

### Live

On main 2688123d's black boxes, `config_v2_scenarios` (`scenarios-v2.md`,
text 0.2), every section in each pairing: **170 of 170**.

| Section | zk2py's owner and caller | the owner example, zk2py's caller | zk2py's owner, `zenctl config` |
|---|---|---|---|
| §1 to §5, §7 to §10, §13, §14 | 44/44 | 44/44 | 44/44 |
| §6 an extend (F-490) | 3/3 | 3/3 | 3/3 |
| §11 sensitive values (F-485) | 4/4 | 4/4 | 4/4 |
| §12 desired state refused (F-482) | 3/3 | 3/3 | 3/3 |
| a re-mint (config.v1 §2.12, F-480, F-481) | 4/4 | 4/4 | not run |

- **§11 step 2:** `zenctl config set` now shows the owner's refusal as
  `device_refused`, with no message (F-485 fixed).
- **§12 step 1:** each owner puts the applied value at seq 1, publishes
  `applied` at seq 2, then puts the report `converged` at seq 3.
- **The re-mint:** with a reach change pending and applied, each owner
  puts the undone record under the old run, nothing pending. It then
  publishes one `rolled-back` event under the old instance number. Then
  comes the new instance's record, revision 0, which T's GET at the new
  token answers.
- **§6:** the deadlines fell 81.20 s, 80.53 s and 80.95 s after step 1,
  and each event came within 3 ms of its deadline.

Also run after the mend, on the same black boxes: `forms_live` 105 of
105, `first_hop_live` 22 of 22, and `desired_scenarios --only 12 --only
config-fed` 16 of 16 (zk2py's `config.v1` owner fed through `desired.v1`,
its re-mint refused and a restart).

**Not run:**
- form 7 live: no owner writes a snapshot (E3), so form 7 is fixtures, the
  echo tool and the readings only;
- the four cases in which config.v1 0.11 says the reference refuses a
  re-mint, against the owner example;
- `live_interop` whole, `link.v2`'s scenarios on routers, and §10's block
  halves, as round 102 said.

Round 103 found F-495 to F-497:
- two minor errors: the fixtures' collection index for zenoh-modem's
  endpoints, 3.2's at 3.3, and Appendix E's `pages`;
- a gap: which revision a reader maps a snapshot's collection index
  with.

None is a blocker. F-480 to F-494 are resolved. See "At core 0.98 (#977
E1, round 102's answers)" in `SPEC-FINDINGS.md`.

## Core 0.99 and `observer.md` 0.27: the bare wire removed (round 104)

Since round 103 main took:
- **core 0.99** (#985 C4d): the bare wire is removed.
  - §5.1's MAY is gone. A reply under another `Encoding` is malformed, and
    an answer all the same.
  - "The request" refuses a query under another `Encoding` with
    `invalid_request`.
  - §5.2's envelope and its Decoding MUST are gone, so a reply error under
    an earlier core's envelope `Encoding`s is the transport's.
  - `errors/cases.json` is removed, and `operations.md` §9's step 7 is
    rewritten, with a new step 8.
- **observer 0.27:** §7.3 loses its bare-wire branch;
- **the profiles, editorially:** telemetry.v1 0.9, config.v1 0.12,
  config.v2 0.3 and redundancy.v1 0.10;
- **#977 E2** (#1078): `conformance/seed/snapshot.json`, a seed over
  snapshots, with no core version and no text (F-498).

zk2py diffed `spec/` and `examples/zk2/` between 2688123d and 4b802646.
**No rule changed beyond what the changelogs state.** No keyword was
added. The E2 fixture is the one change with no entry. Whether
telemetry.v1 0.9 changed a rule turns on how 0.8's citation was read
(F-500).

### The classes zk2py claims

Unchanged: the **owner**, **consumer**, **caller** and **observer**
classes, at the full level, and the static rules. Since core 0.99 a
caller reads a call in the forms alone, and so does an owner.

### Its walk of `RULES.md` at 0.99

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 81 | 70 | 3 | 8 |
| consumer | 23 | 19 | 0 | 4 |
| caller | 18 | 18 | 0 | 0 |
| observer | 93 | 89 | 2 | 2 |
| static, in every class | 68 | 66 | 0 | 2 |

- **254 rows walked of 254, none without a verdict.** 29 verdicts are
  beyond the index.
  - **Gone:** §5.2's Decoding MUST, under the caller and the observer.
  - **New, beyond the index:** three rules core 0.99 and observer 0.27
    state without a keyword. They are the owner's §5.1 "The request", the
    caller's "Under another `Encoding`" and observer §7.3's "Another
    `Encoding`".
- **The re-key came first,** in the commit that brings `just
  py-conformance` level. Rounds 89 and 100 to 103's readings now read core
  0.99, and `seed/snapshot.json` is read.

### What round 104 changed in zk2py

- **The bare wire is gone:**
  - `live.read_reply` reads a reply under another `Encoding` as a refused
    value, its refusal `encoding`, never parsed;
  - `live.read_reply_error` reads a reply error under one as the
    transport's, an earlier core's JSON, CBOR or protobuf envelope among
    them;
  - `live.call` writes only the request form, under `zenoh/bytes`, and
    refuses another `Encoding`. config.v2's caller passes the form it
    writes itself (`written`);
  - `owner.OpCall` refuses a query under another `Encoding`
    `invalid_request`, its bytes never parsed, no handler run, after the
    three key refusals;
  - `calls.bare` is `calls.in_forms`, and `envelope.py` is gone. Its two
    protobuf helpers moved into `health.py`.
- **`seed_snapshot`** (new) reads `seed/snapshot.json` from its
  description alone: a page's entries merged on their members' keys, a
  snapshot complete, and the omission rule with its two guards. That is
  42 cases of 42.
- **`forms_live`:** step 7 is rewritten for core 0.99, and step 8 is new.
  Beyond the step, it sends `set-raw`'s form and bare record under
  `application/octet-stream`, and a wildcard call under `application/json`.
- **The mend** (review-bot on #1076):
  - `config.ConfigOwner.remint` goes in §2.12's order, through public
    Core methods: `end_for_remint`, `startup_values` and `remint`.
    - The device returns before the core restarts.
    - The new instance's view and status go out once its run is taken,
      through `Owner.new_epoch`'s new `before`. Round 103's went out
      first, under the old run, which `desired_scenarios` §12 now checks.
    - A failed declaration puts the old instance back.
    - config.v2's re-mint uses the same hook, with no private access.
  - **config.v2:** a pending change ended before the device refuses its
    return stays ended, as the reference keeps it (#1055). The record
    takes the device's read-back (`Core.returned`). §2.12 says neither
    (F-499).
  - `snapshot.id_bytes`: ASCII in every writer, measure and order, so a
    bad id is always refused `id`.
  - **The nits:**
    - `blackbox.BLACKBOX` alone, read once;
    - §6's window is the text's 81.4 s, its top moved by the extend's
      measured lateness and 50 ms;
    - no `import os` was left unused at main, and the other unused
      imports are gone.
- **`readings`: 953**, up from 937:
  - the bare wire read out of zk2py;
  - core 0.99's and the profiles' entries and counts;
  - §9 steps 7 and 8;
  - F-498 to F-502;
  - the mend.

**`just py-conformance`: 6360 of 6360** on main 4b802646. Of them,
`errors` 316, `seed` 110 and `readings` 953.

### Live

On main 4b802646's black boxes.

`forms_live` (`operations.md` §9): **128 of 128**.

| Step | zk2py's owner and caller | the owner example, zk2py's caller | zk2py's owner, `zenctl call` |
|---|---|---|---|
| 1 to 4 | 37/37 | 21/21 | 31/31 |
| 5 and 6, the test's session | 5/5 | as py-py's caller | 5/5 |
| 7, an earlier core's envelope | 2/2 | as py-py's caller | 4/4 |
| 8, a request under another `Encoding` | 9/9 | 5/5 | 9/9 |

- **Step 7.** zk2py's caller and `zenctl call` each read
  `{"code":"busy","message":""}` under `application/json` as the
  transport's. Each calls `set` once and `ping` twice, then reports
  silence. `zenctl` shows `answer: silent`, with the Encoding and the
  bytes.
- **Step 8.** Each owner answers step 1's form under `application/json`
  `invalid_request`, in one error form under `zenoh/bytes`. So it does
  `set-raw`'s form and its bare record under `application/octet-stream`.
  zk2py's owner runs no handler. Each answers a wildcard call under
  `application/json` `fanout_forbidden` (F-502).
- **The Rust caller.** `zenctl call` writes only the forms, so step 8's
  request under another `Encoding` is the test's session's, in every
  pairing.

Also run, after the mend:
- `config_v2_scenarios`: **172 of 172** in the three pairings. The
  re-mint is py-py 6/6 and rust-py 4/4, with the two refusals of F-499
  new.
- `first_hop_live`: 22 of 22.
- `desired_scenarios --only 12 --only config-fed`: 17 of 17. This adds
  the new instance's status under its new run.

**Not run:**
- step 8's refusal order against an operation not exposed: `tc.v1` has no
  optional operation;
- the four cases in which config.v1 says the reference refuses a re-mint,
  against the owner example;
- form 7 and `seed/snapshot.json` live: no owner writes a snapshot (E3);
- `live_interop` whole, `link.v2`'s scenarios on routers, and §10's block
  halves, as round 102 said.

Round 104 found F-498 to F-502:
- two gaps: `seed/snapshot.json`'s rules have no text, and config.v1
  §2.12's re-mint is refused after its step 1;
- an ambiguity: telemetry.v1 0.9 §2.6's CBOR indefinite lengths and
  trailing bytes;
- two minor errors: the config texts' §2.5 lists, and core §5.1's order of
  refusals.

None is a blocker. F-495 to F-497 stay open. See "At core 0.99 (#985 C4d)"
in `SPEC-FINDINGS.md`.

## Core 0.100 and `observer.md` 0.28: state GETs answered by snapshot (round 105)

Since round 104 main took:
- **core 0.100** (#977 E3, the flip): an owner answers a state GET per
  collection, by snapshot, in pages under 1,024 bytes; a member read
  draws a snapshot of each member, and a concrete key with no member one
  of no entry; the per-key resources keep their reply per key. S4's GET
  is under `None`, merged per key; §4.2's seed applies the omission rule
  with its guards, maps the index by the owner's descriptor (F-497) and
  drops a snapshot heard live. S3's raised window binds only what a
  tombstone alone proves. F-498 and F-502 answered.
- **observer 0.28:** a tool's GET is S4's.
- **freshness.v2 0.6** (#1080): a heartbeat's time from the catalog's
  epoch, all ones outside it, and `heartbeat/times.json`; **0.7:** a GET
  reader ages a snapshot's entry by its confirmation.
- **link.v2 0.9:** §2.11 floors only what a tombstone alone proves.
- **#1084:** config.v1 0.13 and config.v2 0.4 (F-499, F-501, the
  validation fixtures' `encoding`), telemetry.v1 0.10 (F-500).

zk2py diffed `spec/` and `examples/zk2/` between 3c1d33cf and 324c6d4f.
**No rule changed beyond what the changelogs state.** zenoh-modem's
catalog appending `freshness.v2` (#1080) is the example's (F-505).

### Its walk of `RULES.md` at 0.100

| Class | Entries | Met | Not met | Not applicable |
|---|---|---|---|---|
| owner | 88 | 76 | 3 | 9 |
| consumer | 28 | 24 | 0 | 4 |
| caller | 18 | 18 | 0 | 0 |
| observer | 98 | 93 | 3 | 2 |
| static, in every class | 68 | 66 | 0 | 2 |

- **271 rows walked of 271, none without a verdict.** 29 verdicts are
  beyond the index.
- **The re-key came first,** with every new owner and reader rule not
  met, in the commit that brings `just py-conformance` level. The next
  commit implements them and meets them.
- **Not met, new:** observer §4.1's tool GET for the alarms, health and
  config readers, which merge a recording with a GET's entries through
  `seed.merged` and apply no omission rule over what they recorded.

### What round 105 changed in zk2py

- **The owner** (`owner.Owner.state_answer`, `Owner.collections`,
  `owner.per_key`): a collection read's snapshot on its snapshot key, a
  member read's per member, a snapshot of no entry where a concrete key
  wins a collection's template and holds no member; pages by
  `snapshot.cut` under `owner.PAGE_SIZE`; asof the last seq minted under
  the writes' lock; confirmation 1 or 2 with its conf, from the deployment
  epoch. A per-key resource's reply per key, as before.
- **The readers:** `live.get_state` under `None`, each page's entries a
  reply (`StateReply.snapshot_key`, its confirmation), the snapshots in
  `StateReading.snapshots`; `seed.LateJoiner` under `None`, the omission
  rule over `seed_snapshot.omitted` with its two guards, the guard reset
  at each GET's asking, a page heard live counted and never merged,
  `read_again`, and `seed_snapshot.ByDescriptor`, the descriptor whose
  run is the snapshot's (F-507).
- **freshness.v2:** the subscriber re-reads by a collection read and
  replaces its view only from a complete snapshot; `freshness.Snapshot`
  and `reading_of` age an entry by its confirmation (0.7 §2.6).
- **The walk:** `heartbeat/times.json`; freshness's 27 snapshot
  observations; link.v2 0.9's floor (`link.tombstone_only`) and its lints
  with a deployment; config's `encoding`; round 104's open findings read
  as answered; F-503 to F-508's texts pinned.
- **The mend** (review-bot on #1083, and #1084's answers):
  - `seed_snapshot` re-keyed to core §4.2;
  - config.v1's re-mint refuses an act in flight before step 1, waits for
    step 1's undo, then refuses an unknown startup value or a device that
    will not return, the change staying ended (`config_remint_live`, 3 of
    3); the rollback after a failed `new_epoch` is shown whole, since
    `Core.restart` replaces every member it changes (readings);
  - F-471's weak row is gone, and F-473's is a behaviour check;
  - `forms_live` waits for the owner's tokens; `acl_interop`'s blank line.

**`just py-conformance`: 6465 of 6465** on main 324c6d4f. Of them,
`seed` 113, `heartbeat` 163, `link` 315 and `readings` 960.

### Live

On main 324c6d4f's black boxes.

| Runner | py-py | rust-py | py-rust or py-zenctl |
|---|---|---|---|
| `snapshot_live` (state.md §14) | 17/17 | 16/16 | 7/7 |
| `state_pairings` §1, §2, §7, §10, §13 | 46/46 (§1, §2, §7, §10) | 40/40 | all |
| `state_scenarios` §3, §11 | 8/8 | 8/8 (the owner example) | |
| `freshness_v2_scenarios` | 27/27 (§2 to §6) | all | all |
| `heartbeat_epoch_live` | 4/4 (zk2py's owner) | 4/4 (`--catalog`) | |
| `config_remint_live` (config.v1 §2.12) | 3/3 | | |

Also run on the new readers, zk2py's owner unless named: `alarms_scenarios`
(every section, and 80 of 80 against the owner example), `health_scenarios`
31/31, `config_scenarios` 52/52, `config_v2_scenarios` 172/172,
`desired_scenarios` 117 and §9 with zenctl 29/29, `telemetry_scenarios`
57/57, `redundancy_scenarios` 88/88, `budget_scenarios` 27/27,
`hostid_scenarios` 35/35, `freshness_scenarios` 23/23,
`observer_scenarios` 50/50, `forms_live` 128/128, and `live_interop`
(487 checks, its health and seed runs re-keyed to snapshots and run
again, 33 of 33). The re-keyed checks are each F-503's or a GET's order
under `None`. alarms' §6 found an owner bug: a member read gave each entry
the snapshot key to look its bit up by, so a cleared bit read as vouched
for; fixed.

- **§14 in each pairing:** a collection read in 4 pages of 8 (zk2py's
  owner) and 3 pages of 200 entries (the owner example, its records
  `ok`), cut alike by both owners; a member read, `plans/*`'s 32
  snapshots, `plans/v99`'s page of no entry, `oper` and `p9/route`; a
  page lost and read again; `Latest`'s control; a delete beyond the
  window; an empty collection; two instances in either order.
- **The Rust readers on zk2py's owner:** `zenctl get --raw` (4 pages),
  `zenctl get state` (a complete snapshot of 4 pages, a tombstone, a
  member beyond the window, and a page lost: incomplete, pages 0, 1 and
  3) and `consume`.
- **The epoch:** under a catalog of epoch 1767225600 each heartbeat's
  seconds, and the snapshot's time, are the clock less the epoch; a GET
  reader judges `status` fresh under the catalog's epoch and stale under
  0.

**Not run:** an alignment by snapshot (core §4.4): no archive black box;
`state.md` §14 steps 3 and 5's held answer against the owner example,
which cannot hold one, so a session of the test's own plays its answer.

Round 105 found F-503 to F-508: two gaps (alarms.v1's attribution by a
stamp a snapshot does not carry; which descriptor maps a snapshot when two
instances answer) and four minor errors. None is a blocker. F-495 to F-502
are answered. See "At core 0.100 (#977 E3)" in `SPEC-FINDINGS.md`.

## Observer 0.29 and `link.v2` 0.10: a tool across a face (round 106)

Round 106 continues round 105 on one branch, rebased on main 2186bd9f
(#990 V1, V2):
- **observer 0.29:** §7.5 is "Reading across a face". With a face policy and
  its face, a tool's scopes are what the face's plan lets cross, each
  heartbeat's exact key among them; a selector given is narrowed to them,
  and one that meets none is refused. "The revision of a key" picks a
  revision in three steps: the deployment's word, CH's tag, the catalog's
  latest. §7.3's "CH's tag": a record under a tag not its type's is refused
  `tag`, and one under a tag other than 0 whose type is not held is never
  sniffed.
- **link.v2 0.10:** §2.21 points to the tool's scope.
- **The evidence:** `scope/faces.json` (17) and `envelope/tags.json` (31).

No rule changed beyond what the changelogs state.

### What round 106 changed in zk2py

- **`across`** (new): `scopes`, a tool's scopes with or without a face
  policy, the plan `link_v2.plan`'s, what crosses read from its denies on
  `put`, each device of a device face taken as an owner too (F-511);
  `declared`, a selector given narrowed or refused; `revision_of_key`,
  §7.5's three steps and §7.3's tag; `catalog_by_id`, a catalog read and its
  id checked, a predecessor through its successor's lineage (F-509).
- **`echo_tool`:** `--across-face` with `--face-policy` and `--face` takes
  the plan's scopes, and takes a SELECTOR, narrowed or refused; several
  revisions of an interface are held, each sample read with the one chosen;
  a record under a wrong tag is undecodable, `tag`, and one under a tag
  other than 0 whose type is not held is shown as a packed record.
- **The walk:** the `scope` family, `tags.json` in `envelope`, the
  re-worded sentences and RULES.md's 267 rules re-keyed, F-509 to F-511
  pinned. The class walk: 276 rows, observer 98 met, 3 not met, 2 n/a.

**`just py-conformance`: 6523 of 6523** on main 2186bd9f. Of them,
`envelope` 402, `scope` 17 and `readings` 965.

### Live

On the black boxes of 324c6d4f: their owner serves what zk2py's tool reads.
- **`across_live`**, 4 of 4: zk2py's echo across zenoh-modem's RF face
  against the owner example serving `modem.v3` 3.3 at rf0. With no policy it
  hears the owner's heartbeat; through the face its one scope is
  `frames/fast`, and it declares and hears nothing else; a selector over
  `air/**` is refused, exit 2; one over `frames/*` is narrowed.
- `constrained_live` 14/14 and `far_side_live` 16/16 with the new echo.

**On e2bdd8ce's black boxes** (V1, V2 and E4), `across_live`'s reverse half,
2 of 2: `zenctl echo --across-face` holding modem.v3's four revisions and
the catalog, against zk2py's owner putting `packed/oper` and `oper` beside
its heartbeat. With no policy it hears all three, `oper`'s revision chosen
by tag at step 3 and the heartbeat's at step 2. Through the RF face it
declares `frames/fast` alone and hears none of them. After E4,
`snapshot_live` is 40 of 40 and `state_pairings` passes in all three
pairings (50, 40 and 28).

Round 106 found F-509 to F-511: a minor error (`tags.json`'s `e3a529fe` is a
successor's file), an ambiguity (a refused selector, or a refused read), and
a gap (a device face's devices in the tool's plan). None is a blocker. See
"At observer 0.29 (#990 V1, V2)" in `SPEC-FINDINGS.md`.

## What it does not cover

- **The rest of the live half:**
  - archives and alignment (core §4.4, observer §4.5), so `state.md`
    §4, §5, §6 and §9, and §8's union storage;
  - O6's summaries and retries (O4). Call metadata (O7) is sent by the
    client (`live.call(attachment=…)`) and read by the owner
    (`OpCall.metadata()`), for `config.v1`'s attribution;
  - serving roles (§6);
  - constrained faces (§8.5) beyond keys across a face (round 95) and the
    far side's timeouts and declaring again (round 99): a session opened
    again (§8.5's MAY), and a far router's declarations;
  - the scenarios other than presence.md, retrieval.md, grammar.md §1,
    state.md (all but its archives' §4, §5, §6, §9 and §8's union storage,
    round 93) and parts of operations.md. types.md §2 needs a renderer, which zk2py
    does not have: observer §7.3's honest rendering is untested here;
  - §11's History, Archive and union-storage grants, the namespace, and
    §8.5's face declarations toward a far router in a south region: the
    generator emits none of them;
  - observer §3.10's count, and an archive's alignment as a tool sees it
    (observer §4.5, 0.10): zk2py counts no budget and runs no archive;
  - `desired.v1`'s scenario §9, a tool's (`zenctl desired`, #840's DD):
    zk2py's §2.17 runs on fixtures only.
  - of the classes it claims (rounds 89 to 92, and the mend after #926
    part 5): observer §7.3's two MUSTs, decoding and honest rendering by
    the declared type. No zk2py tool holds a contract per key and shows
    its payloads: the echo tool sniffs every payload, each a zk2 key's
    whose contract is not in hand (0.12), but a packed record of a contract
    it is given (0.16), and there is no protobuf or JSON Schema decoder
    field by field and no renderer by declared type. So
    `types.md` §1 to §3 are not run, and §2.4's reliability and §7.2's
    encodings rest on readings (F-379, F-397). Three owner SHOULDs too,
    §1.4's canonical form, §1.5's new session at a re-mint and §3.3's 1 KB.
    S7's consumer rule is kept from the first value since round 91. The
    deployment class is not claimed.
  - a packed record's deployment epoch other than 0, live: the catalog
    states one since core 0.79 (§8.6), and zk2py's owner takes one
    (`Owner.catalog`, `Owner.packed_epoch`) and its echo tool too
    (`--catalog`, `--epoch`), but the live packed pairings run at 0 alone.
    zk2py's echo tool reads no descriptor, so on a host bus it cannot
    resolve the catalog an owner names (F-440).
  - of a `config.v1` owner fed through `desired.v1` (§2.16, round 88): a
    watch session (it reads its author's presence every 500 ms), an
    archive's last-known, and the reference's "wait that cannot end"
    (zk2py's owner waits on a pending change for as long as it pends).
  - a storage manager: a zenoh-python router loads no plugin. state.md §3
    and §12 play a storage's document on the router's own session, so
    observer §4.3's statement (0.48) that every zenoh 1.x storage manager answers
    under `status/plugins/`, never under `config/`, is not measured here.

  zk2py's owner holds values only for parameterless state. It answers
  templated operations through the handlers and members it is given, and
  otherwise names no member.
- **A collection's snapshot** (core 0.97, round 103): zk2py reads, writes
  and cuts pages, and its echo tool shows one. Its owners answer a GET
  per key and its readers read with `Latest`, as S2 to S4 still say: no
  rule adopts the snapshot until E3.
- **A call's forms** (core 0.94, round 101; core 0.99, round 104): zk2py's
  owners answer in them and refuse a query under another `Encoding`, and
  its callers write them and read nothing else. zk2py's tools show a
  call's reply only through
  `live.show_reply`, which `forms_live` judges: no zk2py command-line tool
  calls an operation, as `zenctl call` does.
- **Of config.v2** (round 102): §12's desired document reaches each owner
  directly (`ConfigV2Owner.desire`, the owner example's `config
  desired`), not through `desired.v1`'s binding; no text, enum or
  fixed-point parameter, which 0.1 has not; zk2py's tool for §5 is a
  library (`config_v2.tool_answers`), with no command line.
- **Of `telemetry.v2`** (round 98): a candidate of a `freshness.v2`
  contract with its clock ahead, live (F-461); zk2py's echo tool shows a
  frame's field states, never its series; histograms, which no field
  carries at 0.1.
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
    schemas.py        §7.3 §9.4 artifacts (JSON Schema, protobuf, packed), the subset, $ref, type references
    contract.py       §9.1–§9.5 lints, defaults, canonical form, fingerprint
    sets.py           §9.2      E035, E036
    bundle.py         §9.6      build, verify
    history.py        §9.7      the history check
    descriptor.py     §3.3      the D codes
    catalog.py        §8.6      the deployment catalog: its reading, id, succession and a reader's resolution (round 95)
    cbor.py                     CBOR read and written (cbor's writer, round 101); envelope.py, the bare wire's
                                error envelope, went with core 0.99 (round 104)
    calls.py          §5.1 §5.2 a call's forms by its operation's types: tags, records encoded and read, a packed one
                                under CH's tag, calls.json's writes and reads (core 0.94, round 101)
    zk2c.py           §7.5, observer §7.3 the zk2c envelope: parse, write, which sample carries one, a sample read
                                envelope first (round 93); a call's forms, the request, the value and the error,
                                parsed, written and read by a reply error's Encoding (core 0.92, round 100);
                                form 7, a snapshot, on a put, key_kind off a state key (core 0.97, round 103)
    forms_live.py     §5.1 §5.2 operations.md §9, a call's forms, in three pairings: zk2py's caller and owner, zk2py's
                                caller and the owner example, zenctl call and zk2py's owner (core 0.94, round 101;
                                round 100's run measured a router's own Timeout); steps 7 and 8, an envelope the
                                transport's and a request under another Encoding refused (core 0.99, round 104)
    packed.py         §7.6 §9.2 the packed kind: a file's lints (E038–E045), layouts, the tag, the writer and the
                                reader in exact decimals, a record in the envelope (round 94); a sample read
                                as observer §7.3 reads it, the deployment epoch (the mend after round 94)
    packed_live.py    §7.6, observer §7.3 packed records live: zenctl gen writing for zk2py's consumer, tool and S4
                                GET; zk2py's owner writing for zenctl get state, watch and echo; wrong revisions
                                refused by tag and by length (the mend after round 94)
    compat.py         §9.7 §9.8 the classifier, retention identity
    conformance.py              the runner
    classes.py        §12       the classes zk2py claims, and its walk of spec/RULES.md (round 89)
    live.py           §3.3 §4 §5 §8 presence, descriptor GET, retrieval, state GET, calls; observer §3, §4,
                                §7.2, and §7.4's and §8.2's notes (blind_spots, memlock_note)
    owner.py          §3.3 §4 §5 §6 §8 a minimal owner, a standby (0.39), member tokens (§8.1)
    live_interop.py             the live runner, with the Rust owner and consume examples
    acl.py            §11       the grant generator: a deployment → zenoh's access_control block
    acl_interop.py    §11       the access-control run (`--only acl`)
    security_scenarios.py §11.1 §11.2 scenarios/security.md §4 (0.50, 0.51) on zk2py's block and zenctl acl gen's,
                                zenctl get as the tool (observer §4.3), and §11.2's inclusion facts (`--only security4`)
    hostid.py         hostid.v1 the derivation, the shape, the runtime over a root
    hostid_scenarios.py hostid.v1 scenarios.md §1–§6 in temporary roots
    freshness.py      freshness.v1 horizons, judgements, the clock trust, a subscriber, a tool's read
    freshness_scenarios.py freshness.v1 scenarios.md §1–§6 on an in-process router
    heartbeat.py      §7.5      the heartbeat: the tag, the collections, the body written and read, the digest
                                (round 96)
    snapshot.py       §7.5      the snapshot, form 7: a page read, written and written back, cut by the paging
                                rule twice (by writing, by arithmetic), held complete (core 0.97, round 103)
    seed_snapshot.py  §4.2      a seed over snapshots, core 0.100's §4.2: the fixture's Seed, and the live
                                reader's pages, entries, Tracker, omitted and ByDescriptor (round 105)
    frame.py          §7.7      frames: the declaration, the frame form and its tag, the body written and read
                                over packed's fields (round 96, core 0.83)
    freshness_v2.py   freshness.v2 §2.5's rule (the view), a subscriber that re-reads, the owner's body (round 96)
    freshness_v2_scenarios.py freshness.v2 scenarios-v2.md §1–§7 in three pairings: zk2py's owner and readers, the
                                Rust owner with zk2py's, zk2py's owner with zenctl watch and get state (round 96)
    health.py         health.v1 levels, §2.11's reader, roll-ups, codes, the payloads, the owner half
    health_scenarios.py health.v1 scenarios.md §1–§8 on in-process routers
    config.py         config.v1 time, values, the orders of refusals, desired state, redaction, Core,
                                ConfigOwner (fed through desired.v1 too, its re-mint a restart or refused), SimDevice,
                                the reader of §5
    config_scenarios.py config.v1 scenarios.md §1–§14 against zk2py's owner or the Rust owner example
    config_v2.py      config.v2 the declaration and its reasons, the derived types and their layouts, the contract's
                                lint, the record written and read, validation, times, the owner (Core, its live
                                LiveCore and ConfigV2Owner), redaction, a caller and a recorder, §5's tool, and
                                zenoh-modem's SX1262 round trip (round 102)
    config_v2_scenarios.py config.v2 scenarios-v2.md §1–§14 in three pairings (zk2py's owner and caller, the owner
                                example's --config-v2 with zk2py's caller, zk2py's owner with zenctl config), and a
                                re-mint (config.v1 §2.12) with both owners (round 102)
    link.py           link.v1   the face policy, exposure, the plan and its carve-out, the checks, a generator
    link_scenarios.py link.v1   scenarios.md §1–§13 on zenoh-python routers, over examples/zk2/link/
    link_v2.py        link.v2   the policy, the plan, the transport block and its checks (round 96); the contract
                                side and frames per face, exposure.json (0.3, round 99)
    far_side.py       §8.1 §8.5 a far side across a face: the reply timeout, a subscription's horizon, Redeclaring
                                (core 0.91, observer 0.23, link.v2 0.4; round 99)
    far_side_live.py  §8.1 §8.5 constrained.md §8 and §9 in process, a proxy dropping a batch the far client sends
    adopter_frames.py §7.7     modem.v3 3.2's and gendev.v1's frames written, read back and read as series (round 99);
                                link.v1 0.16's largest frame, its delta records, measured (round 100); modem.v3
                                3.3's three over Readings beside 3.2's nine, deprecated (#991, round 102)
    alarms.py         alarms.v1 the key and the sanitizing step, references, strict JSON, the document, transitions,
                                the conclusions, AlarmsOwner, the reader on a bus
    alarms_scenarios.py alarms.v1 scenarios.md §1–§10 against zk2py's owner or the Rust owner example
    telemetry.py      telemetry.v1 the standard schema's ids, declarations, carriers, a sample's conformance,
                                a pair's epoch, continuity and rate, a contract's and a bundle's resources
    telemetry_scenarios.py telemetry.v1 scenarios.md §1–§9 against zk2py's owner or the Rust owner example, §8 across a face too,
                                and zenctl telemetry judging beside zk2py's reader
    telemetry_v2.py   telemetry.v2 a frame's fields as series: §2.16's declaration, §2.18's readings, §2.19–§2.21's
                                pairs (the run as the epoch, the serial window, the time base and its basis), §5 over
                                many pairs, the owner's meter (round 98)
    telemetry_v2_live.py telemetry.v2 scenarios-v2.md's R on a bus: one series reader per frame key, the bare
                                metric streams by v1 with no presence (round 98)
    telemetry_v2_scenarios.py telemetry.v2 scenarios-v2.md §1–§7 in three pairings: zk2py's owner and R, the Rust
                                owner with zk2py's R, zk2py's owner with zenctl watch, get and telemetry (round 98)
    desired_live.py   desired.v1 the author and the target on a bus, the scenarios' drive
    desired_owner.py  desired.v1 either party as a process, driven as the owner example's --desired-* are
    desired_scenarios.py desired.v1 scenarios.md §1–§12, edges, a re-mint and the tool's shapes, with any two
                                parties, zk2py's tool and zenctl's; §12 and config-fed with zk2py's config.v1 owner
    desired_tool.py   desired.v1 §2.17's tool on a bus: the slice, the report, the author's version, a window
    desired.py        desired.v1 the standard schema's ids, declarations, bindings and the slice's key, stamps, equal
                                documents, the target's state machine, the report's reader, §2.17's conclusions
    budget.py         §2.7, observer §5 the population budget: the bound, a population, a rate, lossless, the stall watch,
                                readings on a bus
    budget_scenarios.py §2.7, observer §5.2 scenarios/budget.md §1–§7 and §1's stalled reader: the measurements, and the owner keeping its budget
    seed.py           §4.2, §4.3, observer §4.1 a late joiner's seed: subscribe, then S4's GET; a consumer's by S7
                                from its first value, a tool's by the merge for as long as it follows, bounded,
                                its count reported (bound_note); the pure merge, and the readers' (merged)
    presence.py       observer §3 what a complete presence read is (0.37, 0.41): the view (a withdrawal unconfirmed until a
                                read, a read unordered while no round trip stands, a transport's move) and the follower
                                on a watch session
    presence_scenarios.py observer §3 scenarios/presence.md §1–§10 (0.41) with either owner and either reader under test
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
                                each member's answers, each resource's and the run's, across a face too; its scope
                                and its clocks (observer §8.2, §8.3)
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
    state_scenarios.py §4.2, observer §4 scenarios/state.md §3 and §12 on a router process (zk2py's owner or, with
                                --owner rust, the owner example), §11's open cases, and state_pairings' sections
    state_pairings.py §4, §7.5 scenarios/state.md §1, §2, §7, §10 and §13 at core 0.73, in three pairings: zk2py's
                                owner and readers, the Rust owner with zk2py's, zk2py's owner with zenctl and consume;
                                re-keyed to snapshots at core 0.100 (round 105)
    snapshot_live.py  §4.2, §7.5 scenarios/state.md §14 at core 0.100, in three pairings, over
                                interop/state/zk2py_fleet.v1.toml (round 105)
    heartbeat_epoch_live.py freshness.v2 0.6 §2.4, core §4.3: a heartbeat's and a snapshot's time under a
                                catalog's epoch, the owner example with --catalog and zk2py's (round 105)
    config_remint_live.py config.v1 0.13 §2.12: the re-mint's refusals in order, and its wait for step 1's
                                undo, live (round 105)
    across.py         observer §7.3 §7.5 reading across a face: the scopes by the face's plan, a selector
                                given, the revision of a key in three steps, CH's tag (round 106)
    across_live.py    observer §7.5, live: zk2py's echo across zenoh-modem's RF face against the owner example
                                (round 106)
    echo_tool.py      observer §8.1 §8.2 a generic watcher: the samples a selector delivers, and its scope, what it watches
                                and the verbatim kinds it excludes, in its report and on stderr; NDJSON rows with the
                                scope on stderr alone; each payload sniffed, its row saying which case and where it
                                stopped resolving the key (observer §7.3, 0.12); with --contracts, a packed record
                                decoded or named undecodable (0.16); --across-face by observer §7.5's scopes, and
                                --catalog's epoch (round 95)
    face.py           §8.5, observer §7.5 a resource's scope across a constrained face: its exact key or its
                                template's selector (round 95)
    constrained_live.py §8.5   constrained.md §7 live, zk2py's owner, tool and consumer through a byte-counting
                                proxy, and an owner given a catalog (round 95)
    first_hop.py      §8.1 §8.4, observer §3.11 a tool's report of an owner on a first hop: found by its instance
                                token and descriptor, its link closed and never down (core 0.96; round 102)
    first_hop_live.py §8.1 §8.4 constrained.md §10 live, a first-hop device through a byte-counting proxy, the
                                owner example's --first-hop or zk2py's Owner(first_hop=True) (round 102)
    decoding.py       observer §7.3 the decode order: a held JSON Schema type on its Encoding's wire, sniffing, and what a
                                tool says of a payload whose declared type it does not hold (0.12)
    judgement.py      observer §1 the judgement shape: a profile's answer read as Established, NotAsked, Unobservable or not
                                this profile's (either spelling), and its cell, field and note
    observer_scenarios.py observer §3.2 §7.2 §8.2 core 0.59's three steps: grammar.md §1 (step 5's tools),
                                operations.md §5 steps 1-2 under generated grants, §8 step 6, with zenctl beside
    admin_records.py  observer §4.3, §11.1 the keys a node answers under its tables' read lock, shared by live and acl
    adminspace.py     observer §4.3, App. B a router or a peer in a process of its own, the load beside it, the @/** recorder,
                                and the freeze measurement (never a suite), under access control too (0.50)
    blackbox.py                 where the live runners find the reference's black boxes: $ZK2PY_BLACKBOX, else
                                /srv/dev/scratch/zenkey/bin/current (the mend, round 103)
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
                      arbitration.v1's scenarios' contracts; grammar/ zk2py_guard.v1, grammar.md §1's owner;
                      operations/ zk2py_frozen.v1, operations.md §5's frozen server; tc.v1 with calls.json
                                  and proto/tc/v1/calls.proto, operations.md §9's contract (round 101);
                      desired/ also configs.v1 (revision 1.1 on freshness.v2 since round 98) and
                      zk2py_configured.v1, desired.v1 scenarios §12's author
                      and the role of zk2py's config.v1 owner; classes/ zk2py_replica.v1, three operations
                      for the owner rules round 89's walk found (§5.1, §5.2, O6); state/ zk2py_ground.v1,
                      state.md's plans, status and streams at 0.73, and zk2py_relay.v1, §13's governed stream;
                      packed/ zk2py_packed_reader.v1, the consumer of the Rust tests' packed.v1;
                      constrained/ zk2py_face.v1 and zk2py_face_reader.v1, constrained.md §7's owner and consumer,
                      and zk2py_faced_reader.v1, §9's consumer of the reference's faced.v1
```

The JSON schemas are read from `spec/` at run time (`shape.py`), not copied.
So the E000 and D000 checks follow the published schema files exactly.
