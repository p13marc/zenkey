# `desired.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.7 — 2026-10-25: round 61 (#840, DF; #609)

zk2py read 0.6 cold in round 61, at PR #864's head, and found every
earlier finding resolved. It filed one, F-224 (minor), which #840's
coordinator accepted. It is answered here, beside review-bot's points on
PR #864, which settle what a requirement naming no resource names. 0.6
stays as the round read it.

The evidence, counted from the files: `conformance/declarations.json` (43
cases), `conformance/bindings.json` (29), `conformance/order.json` (88),
`conformance/reports.json` (80), `conformance/conclusions.json` (35),
`conformance/schema.json` (3) and `conformance/slices.json` (31), 309 in
all; and `scenarios.md`'s eleven sections, §11 new. No case that existed
changes. `slices.json` gains two cases.

**Answered.**
- **F-224, the target's refusal had no evidence of its own** (§2.1). 0.6's
  "A target MUST refuse such a role at its start" cited `bindings.json`,
  which states the reading, `two_documents`, not the act: a target that
  reads the binding right and starts anyway passed every case, and
  "refuse such a role" could be read as "start without it". Now, in
  zk2py's words: a target given a role on two or more desired resources of
  its author's declaration MUST refuse to start, as core §8.2 step 2
  refuses an invalid declaration: nothing is declared and no instance
  token of it appears. `[Sc: §11]`: the target given such a role shows no
  instance token within 5 s, by liveliness at S, against a control whose
  requirement names one of the two, which starts. The reference refuses
  the start, not the role alone: a builder whose role was refused refuses
  `start()` too, `not started: …`, as the owner example says on exit
  (`zenkey/tests/profile_desired.rs`'s `s11_…`, and the owner example's
  test).
- **What a requirement naming no resource names** (§2.1, review of PR
  #864). Core §3.1's default is every resource, and a role a component's
  manifest declares has no `resources` a tool can read, its descriptor
  naming no declaring contract (core §3.3). Both name every resource of
  the interface, so on an author with two desired resources either is
  `two_documents`; with one desired resource beside plain ones, the one is
  the slice. The reference's runtime reads a manifest's role so too, and a
  lookup that fails is an error, never every resource. `[F: slices.json]`:
  a requirement naming none on two desired resources, and on one beside a
  plain one.

**Deliberately not changed.**
- **§2.3's order.** `two_documents` left unbound reads as the kill switch
  (optional) or `required_unbound` (required), as `bindings.json`'s two
  cases of 0.6 already state.
- **§2.17 and §5**, their tokens, and every fixture that existed.

## 0.6 — 2026-10-25: round 59 (#840, DF; #609)

zk2py read 0.5 cold in round 59 (PR #861): every case passed, and the two
tools agreed everywhere, on two shapes the text did not state. It filed
three findings, F-217 to F-219, which #840's coordinator accepted
(issuecomment-35724). F-217 and F-218 change the reference's reading and
its runtime; F-219 is the reference tool's help.

The evidence, counted from the files: `conformance/declarations.json` (43
cases), `conformance/bindings.json` (29), `conformance/order.json` (88),
`conformance/reports.json` (80), `conformance/conclusions.json` (35),
`conformance/schema.json` (3) and `conformance/slices.json` (29), 307 in
all; and `scenarios.md`'s ten sections. No case that existed changes.
`bindings.json` gains an input, `also`, the other desired resources of the
author's declaration the requirement names, and four cases;
`slices.json` gains an answer, `{found: not_a_target}`, and seven cases.

**Answered.**
- **F-217, a role on no desired resource** (§2.1, §2.17, §5). A target's
  roles are not all on desired resources: a worker may also consume its
  author's plain state. 0.5 said what a tool answers when the revision its
  descriptor names declares no desired resource (both tools answered
  `not_a_target`), and nothing when only a held revision showed it: the
  target-alone rule then read the role as a candidate, `no_report`. Now a
  role whose author's revision in hand, named or held, declares no desired
  resource the requirement names is not this profile's, **not asked**,
  `not_a_target`, and the target alone is read only with no revision of
  the interface in hand at all. The reference's reading changes: a held
  revision showing the resource not desired gives `not_a_target`, never
  `no_report`. `[F: slices.json]`: the named revision and a held one each
  declaring the resource with no `desired.target_param`, and a held one
  declaring no resource the requirement names.
- **F-218, a requirement naming two desired resources** (§2.1, §2.3,
  §2.17). 0.5 let a requirement name several templates from the target
  alone (F-210), but with the author's contract in hand took its first
  desired resource, by an order no rule states: one report answered or
  not as the tool held the contract or not. Now a requirement on desired
  resources names exactly one, §2.1's "one role per document type", and
  one naming two or more, as the author's declaration shows, is an invalid
  binding, `two_documents`, in §2.3's list after `required_unbound`
  (which reads only whether a binding exists, leaving the kill switch the
  deployment's) and alone (every later problem binds the one slice there
  is not). It gives no slice: **unobservable**, `no_report`, the problem
  beside. From the target alone it cannot be seen, and every template
  stays a candidate. A target MUST refuse such a role at its start,
  nothing of it declared, as core §8.2 step 2 refuses an invalid
  declaration: the reference does (`zenkey::desired`), and its model
  reads `two_documents` from a requirement's count of desired resources
  (`zenkey_model::desired::Requirement::documents`). `[F: bindings.json]`:
  two documents, two documents beside two authors, and the order against
  `required_unbound` and the kill switch; `[F: slices.json]`: the named
  revision and a held one each declaring two, the order, and the same
  requirement from the target alone.
- **F-219, `zenctl desired --help`** (the reference tool) said 0.4's rules.
  It now says 0.5's (every template under either kind token from the
  target alone, the report's key choosing; a version dated at its slice
  GET's answer) and 0.6's (`not_a_target`, `two_documents`).

**Deliberately not changed.**
- **The fixtures that existed**, and `conclusions.json`'s inputs: a role
  no target's reaches §2.17 as a descriptor not listing it, and
  `two_documents` as no report.
- **§5's four questions** and their tokens, and the scenarios.

## 0.5 — 2026-10-24: round 57 (#840, DE; #609)

zk2py read 0.4 cold in round 57 (PR #857) and filed four findings, F-210
to F-213. #840's coordinator accepted all four (issuecomment-35632). Each
is text: the reference already behaves as the answer says, so the rule is
written in, and a new fixture pins it, which every implementation runs.
The same chunk carried review-bot's points
on PR #856 (issuecomment-35609), which change the reference's reading and
none of this text's rules: the reads of one reading are made at once, a
version's age is taken when its GET was answered, and `check conform`'s
`desired-report` reads the interface's own reports.

The evidence, counted from the files: `conformance/declarations.json` (43
cases), `conformance/bindings.json` (25), `conformance/order.json` (88),
`conformance/reports.json` (80), `conformance/conclusions.json` (35),
`conformance/schema.json` (3) and the new `conformance/slices.json` (22),
296 in all; and `scenarios.md`'s ten sections. No case that existed
changes.
- **`slices.json`** (new) is a tool's reading of one role before §2.17
  answers, its input as data: the target's requirement (templates,
  bindings, parameters), the author's contract as the tool holds it (the
  revision its descriptor names, or revisions held offline, each its
  resources as `declarations.json` gives them), and the replies of the
  target's report resources (each a key and a payload's text). It expects
  the candidate slices, where the slice came from (`author_contract`,
  `offline_contract`, `requirement`), §2.3's binding problems and §2.2's
  declaration reasons, and which reply is the report: one (the slice it
  claims), unreadable (`two_differ`, `unreadable_reply`), none, or no
  slice. Its 22 cases pin F-210, F-212, F-213 and 0.4's §2.10 and §2.17
  rules. The reference runs it in `zenkey_fleet::model::desired`;
  `zenkey-model`'s profile test checks every case's shape and the slice
  of each case whose author's contract is in hand, by the model's
  declaration and binding.

**Answered.**
- **F-210, the template a requirement names** (§2.17). 0.4 took "the
  template its requirement names" as one. Now, with the slice from the
  target alone, every template the requirement names, under either kind
  token, is a candidate; the one a readable report of the target claims
  is the slice, and with none claimed the answer is `no_report`. A
  requirement naming one desired resource, as every example does, has one
  candidate per kind token. `[F: slices.json]`: two templates, the
  report's key choosing among four candidates, none claimed, and two
  claiming two candidates and differing.
- **F-211, how long behind** (§2.17). "In readings that long apart" read
  as two readings. Now: in every reading over that long, the first and
  the last that long apart, any other answer between them, an
  unobservable one included, starting the window again. That is the
  reference's `--for` and §9's step 6. `[Sc: §9]`
- **F-212, two reports on one key** (§2.10, a minor). "Two readable ones,
  of two report resources, that claim it and differ": two instances'
  reports on one key are one reply to S4's GET, the later (core §4.1), and
  §2.17 reads a split target's report as the later one's.
  `[F: slices.json]`: two resources' reports that differ, two equal ones,
  and the one reply on a key.
- **F-213, a revision held offline** (§2.17). 0.4 named "one held
  offline" without saying which. Now, with no revision its descriptor
  names in hand, a revision of the author's interface held offline that
  declares a desired resource the requirement names; one major's
  revisions keep each resource's key (core §9.8), so any such revision
  gives the same slice, and the slice so taken is the slice: a report
  keyed under the other kind token is none, the contract's kind token
  winning over the report's. `[F: slices.json]`: a held revision with no
  descriptor, a report under the other kind token, a held revision
  declaring the resource explicit, and the one of two held revisions that
  declares it.
- **The clock** (§2.17, with review-bot on #856): a version's age is taken
  when its GET was answered, never the reading's end, which is later.

**Deliberately not changed.**
- **The fixtures that existed.** `conclusions.json` keeps its inputs: a
  reading's outcome before §2.17, which `slices.json` now states, is one
  of its inputs, no report or `undecodable`.
- **§5's four questions** and their tokens, and the scenarios.

## 0.4 — 2026-10-24: round 55, and the tools (#840, DD; #609)

Two sources. zk2py ran 0.3 live in round 55 (PR #853), every pairing of
it and the reference as author and as target passing §1 to §8 and §10,
and filed seven findings, F-192 to F-198, each accepted by #840's
coordinator (posted on #840). And the reference's tools were built
(`zenkey_fleet`'s `desired` reading, `zenctl desired`, the doctor's
`desired-*` checks and `check conform`'s `desired-*` cases), which ran
§9 for the first time. Written against core 0.39: 0.38's read cut off
while it runs is possibly incomplete (§2.5, §2.17), and 0.39's standby
changes none of this text's rules.

The evidence, counted from the files: `conformance/declarations.json` (43
cases), `conformance/bindings.json` (25), `conformance/order.json` (88),
`conformance/reports.json` (80), `conformance/conclusions.json` (35) and
`conformance/schema.json` (3), 274 in all; and ten scenario sections, §10
two steps longer. No expectation of a case that existed changes: the
eight new cases of `order.json` were written by hand and passed as
written, and its description gains the input they need. The standard
schema is unchanged, and so is its id.

**Answered.**
- **F-192, "in an instance" against "a re-mint is not a restart"**
  (§0, §2.6, §2.7, §2.10, §2.16, §5). 0.3 kept a target's acts "in an
  instance", and said a re-mint, which changes the instance, keeps them.
  §0 now defines a **run**, one process of the target: its acts, the ids
  it remembers and its last act are kept in one run; a re-mint keeps the
  run, a restart begins another. Every "in an instance" that meant this
  reads "in a run", and "a restart acts again" reads "a new run". The
  author's "instance" (§2.11) is unchanged: a re-mint is a new instance of
  an author, and puts again what it keeps. `order.json`'s description
  says "run", and `{restart: true}` is a new run.
- **F-193, the pointer of a refusal no member carries** (§2.7). Stated,
  as JSON Schema's output places an error: a keyword of a member's own
  schema at the member; a required member missing at the object that
  holds it, `""` at the top; a value of the wrong type at that value. A
  document the schema refuses in more than one place is refused at one of
  them, the target's choice.
  - `order.json` gains its input, `{"finish": "check"}`: the running act's
    put ends as the fixture's own type judges its document (a top-level
    `schema`, `$defs.Task`: `tasking.json`'s `Task` and a member object
    `limits` with a required `max_hz`), `invalid` at that pointer. Eight
    cases: one admitted, and one per rule, each breaking the schema in one
    place (`""` for a missing `rate_hz`, `/rate_hz`, `/mode`, `/limits`
    missing `max_hz`, `/limits/max_hz`, `/limits` of the wrong type, `""`
    for an array).
  - The runtime already placed them so (`zenkey_model::validate`, the
    first violation's pointer); zk2py's runner needs the new input.
- **F-194, the owner example's `desired put` is not "as written"** (the
  example). The line is put byte for byte once it reads as strict JSON
  (§2.10's reading), so every number keeps its exact value (F-166,
  `order.json`'s case of 2^53 + 1); the help says so, and
  `owner_example.rs` puts a number past a double's precision.
- **F-195, "(core §8.1, 0.3)"** (`scenarios.md` §7, §10): "(core §8.1,
  0.37; this text 0.3)". F-182's slip again.
- **F-196, "the timestamp s2, exactly"** (`scenarios.md` §2). X builds
  every timestamp from the one S received with `s2`: a binding may not
  build every NTP64 time from its text, and `s2`'s fraction must survive.
- **F-197, an archive read only on absence is never observed** (§2.5,
  `scenarios.md` §10).
  - §10 gains steps 3 and 4: the link cut, the author puts v2, X holds an
    instance token at the author's address on the target's side and no
    queryable, and the target restarts: its author present, its GET
    silent, the archive holding v1. The reference reads no archive and
    acts on nothing; a target taking §2.5's MAY on a silent GET acts on
    the archive's v1, which its author replaced, and the text names that
    as what the reference avoids. Then the heal, and v2 converged.
  - The owner example says what a seed found (`desired seeds`: the replies
    its GET drew, the author's presence or why the read was not made, the
    archive), so §2.5's "says what its seed found" is observable from
    standard input; and an author how many GETs reached a target's slice
    (`desired gets`), for §8's "the author's queryable receives none".
- **F-198, a paced author's kept document and core S2's "live"** (§2.11).
  A document the new instance keeps is live from its put by that
  instance: until a paced author has put it again, its GET answers
  without it, and a tool reads `author_silent` for that target, never
  behind. Core S2 is unchanged.

**Changed: what building the tools showed.**
- **The slice without its author's contract** (§2.17). The slice's kind
  token and target parameters are the author's declaration, and an author
  absent (§9's step 4) has no descriptor naming its revision; nobody else
  serves its bundle. 0.3 did not say how a tool finds the report then. A
  tool MAY take the slice from the target alone: its requirement's
  template, its binding's parameters, the kind token the report's key
  carries. The reference does, and says where each slice came from
  (`slice_from`: the author's contract, one held offline, or the
  requirement).
- **Which reply is the report** (§2.10). Two readable reports claiming one
  slice and differing are unreadable; with none readable claiming it, one
  that does not read is unreadable too, since it may be the slice's.
  Otherwise none was read. 0.3 said only "the one whose key is its key".
- **No slice, no report** (§2.17). A binding §2.3 refuses, or a
  declaration §2.2 refuses, gives no slice: `no_report`, the problems
  beside, never a guessed key.
- **An unstamped reply of the author's** carries no version (core S1):
  `author_read_incomplete`. **A standby author** (core §6, 0.39) answers
  no GET: `author_silent`.
- **How long behind** (§2.17, `scenarios.md` §9 step 6). "A tool MAY
  report a target behind for longer than it expects" now says how: never
  from one reading alone; from readings that long apart, the author's same
  version current and the target not yet on it; or, on a clock trusted to
  the HLC delta, from that version's stamp. §5's answer stays not yet.
  The reference's `zenctl desired --for` reads again every 500 ms and
  exits 1 on such a target; the doctor's `desired-behind`, one reading,
  finds one only on `--clocks-synced`.
- **`scenarios.md` §9** names the reference's T and its exits, the slice
  taken from the target when the author is absent, and a sixth step: one
  reading not yet, a window behind.

**Deliberately not changed.**
- **The fixtures.** Each rule above is one of the reading, before §2.17
  answers: its outcome is an input `conclusions.json` already has (no
  report, `undecodable`, `incomplete`, `silent`), and every one of its 35
  cases reads through the reference's projection as written
  (`zenkey_fleet::judge::desired`'s test). "Behind past a window" is a
  tool's finding beside §5, not one of its answers.
- **§5's four questions** and their tokens.
- **No report member for the window** or the slice's source: the report
  is the target's; what a tool read is its own.

## 0.3 — 2026-10-24: the runtime, and round 53 (#840, DC; #609)

Two sources. zk2py read 0.2 cold in round 53 (PR #845): every fixture
passed, and it filed four findings, F-181 to F-184; round 54, reading core
0.37 cold, filed F-187, whose `desired.v1` half is answered here (its
`alarms.v1` half is not this text's). The answers are #840's
coordinator's, posted on #840. And the reference's runtime was built
(`zenkey::desired`: an author's writer and record, a target's reader,
`config.v1` fed through it, and the owner example's `--desired-*`), which
found what 0.2 did not say. Written against core 0.37: §2.5 and §2.9 now
read presence as its §8.1 does.

The evidence, counted from the files: `conformance/declarations.json` (43
cases), `conformance/bindings.json` (25), `conformance/order.json` (80),
`conformance/reports.json` (80), `conformance/conclusions.json` (35) and
`conformance/schema.json` (3), 266 in all; and ten scenario sections, of
which the reference runs §1 to §8 and §10 (`zenkey/tests/profile_desired.rs`,
one test per section). No expectation of a case that existed changes: the
eight new cases were written by hand and passed as written, and the two
descriptions they extend say what they pin. The standard schema is
unchanged, and so is its id.

**Answered.**
- **F-181, a report's number: exact, or a double** (§2.10, §2.4). 0.2
  said when a report's number is finite, and what a document's number is
  when two are compared, but not what a report's number is once it reads.
  Now every number is read by its exact value, as §2.7 compares a
  document's: a number with a zero fraction is that integer, so
  `18446744073709551615.0` is 2^64−1, a revision in range, and `1.5` is no
  integer. This is JSON Schema's reading of `integer`, which core §7.3 does
  not narrow; a document's own revision (§2.4) is read the same way.
  - The model read the double: `decode` now reads a revision from the
    report's text (`zenkey_model::desired::decode`), and
    `revision_of_text` a document's. `Report::from_json`, given a decoded
    value, can only read the double it holds, and says so.
  - zk2py's reading, the double, flips.
  - `reports.json`: its description, and five text cases:
    `18446744073709551615.0` in range, `18446744073709551616.0` past it,
    `1.5` outside the schema, `9007199254740993` beside
    `9007199254740993.0` one revision with no `in_force` finding, and
    `30e-1` the integer 3; and, on review of PR #850,
    `1e99999999999999999999`, an integer far past 2^64−1 by its exact
    value and never finite, so `json`: strict JSON is read before the
    schema. The model reads a huge positive exponent as too large, never
    as a fraction, where it is ever asked.
- **F-182** is `config.v1`'s: its text 0.5 moves the marker.
- **F-183, scenario §10 cited by no rule** (§2.9; core Appendix C).
  §2.9's "Confirmed by the author" and "From an archive" cite `[Sc: §10]`,
  and core Appendix C's row lists §10 (an index entry, recorded under core
  0.37's entry: the core's version does not move).
- **F-184, what "holds waiting" confirms** (§2.6 step 3). The text, read
  literally, confirmed the author's own version held waiting; the
  fixture's description confirmed only a version from an archive. The
  description is the rule: step 3 now says "holds waiting from an
  archive", and that any other delivery of a version held waiting, the
  author's own included, is the same version. The model already read it
  so. `order.json`: its description, and two cases, the author's own
  version held waiting received again from the author, and from an
  archive, each the same version.
- **F-187, "gone" against core 0.37** (§2.5, §2.9, §2.17). §2.5 let a
  target read an archive "with its author's instance token gone"; core
  0.37 §8.1 makes a withdrawal heard live a reason to read again, never
  absence. Now the author is absent by a complete read without its
  instance token, in §2.5, §2.9 and §2.17's steps 1 and 6, and §5's
  table; a read through a session cut off is not made, and §2.17 counts
  it beside the possibly incomplete. Every other "absent" or "gone" in the
  text was read again: the rest are a report's absent member, or a
  process gone.

**Changed: what building the runtime showed.**
- **Reading presence** (§2.5). A target that follows its author live
  holds the liveliness subscriber on a watch session of its own, makes one
  round trip there before its first seed, and reads presence on another
  session (core §8.1, 0.37); one that does not reads on a period, and the
  text says what that misses: an absence shorter than the period whose
  author returns as the same instance, and any version put during it,
  until the deployment's re-read. The reference's period is 500 ms. A
  first version polled only after its first sleep missed a re-mint within
  its first period, which §4's step 3 found: it reads at once.
- **A seed at an appearance that draws nothing** MAY be made again: the
  instance token can reach the target's router before the route to the
  author's queryable, as an archive's alignment found (core §4.4). The
  reference retries four times.
- **The reference's last-known** (§2.5, §2.9). It reads an archive only
  after a complete read shows the author absent, never on a silent GET
  while the author is present: an archive can hold a document its author
  deleted past its window (§2.8). The text's MAY on silence stands.
- **`undecodable` and `invalid`** (§2.7). A document that decodes and
  that its JSON Schema refuses is `invalid`, its pointer where the schema
  refuses it: decoding is core §7.2's, which the schema is not.
- **An act whose end is unknown** (§2.7) is `failed`, code `apply`: the
  reference runs each act on a task of its own, and one that panics may
  have applied anything.
- **A re-mint is not a restart** (§2.7). A target re-minted in its own
  process still runs what it applied, and keeps its acts and its report;
  only a new process acts again.
- **The author's restart** (§2.11). Its record holds each delete with
  when it was made, and each entry's stamp for the clock's catch-up (core
  §4.3); a delete made again is answered for what is left of its window,
  not a window more, and drops from the record when that has passed; and
  a paced re-put makes its deletes first, then its documents at a rate,
  the MUST of "before a GET is answered" kept by the store answering only
  what the new instance put, and a document the author puts meanwhile
  never put over.
- **`config.v1` as a target** (§2.16, Appendix B). What a delete reverts
  an owner to, which 0.2 left to the runtime: each parameter a document
  it acted on named, a refused one's aside, back at its startup value,
  through one `set` per group as converging does, its view and status
  then carrying no `desired` (`config.v1` text 0.5); a revert that fails
  keeps the parameters it did not restore for the next delete (on review
  of PR #850). And, fed through this profile, the owner acts on each act
  it is given, whatever the revision; a wait on a pending change that
  never ends is bounded in the reference (5 s past its deadline, or the
  service's close), the act `failed`, `apply`, since one act at a time
  would otherwise hold every later version.
- **scenarios.md** names the owner example's options (`--desired-author`,
  `--desired-target`, `--desired-unbound`), and §7 and §10 say that the
  author's absence is a complete read's.

**Deliberately not changed.**
- **A lost delivery** (§2.5's SHOULD). zenoh 1.10.1's subscriber does not
  say what it lost, so the reference cannot seed on it; the deployment's
  period covers it, as the text already says for a face.
- **No report member for a read not made.** The target's report says
  what it did; what a seed found, a read not made included, is the
  runtime's to log and expose (`zenkey::desired::Target::last_seed`),
  never a field of `desired.v1.Report`, whose schema is unchanged.
- **The MAY on a silent GET** (§2.5, core S6) stands for an
  implementation that wants it; only "gone" is read as core 0.37 says.
- **§9** is a tool's, and stays #840's DD.

## 0.2 — 2026-10-24: the cold reader's round 52 (#840, DB; #609)

The spec-only implementation read 0.1 cold with core 0.36 (zk2py round
52, PR #843): every fixture passed the first time it ran, 222 cases and
64 report read-backs, and it filed twenty findings, F-161 to F-180. Three
are fixture expectations that decided what the text left open or said
otherwise (F-161, F-164, F-165); two are readings no case told apart
(F-162, F-163); five more are rules a reader had to guess (F-166 to
F-170); two are `config.v1`'s (F-171, F-172); two are the record's (F-173,
F-174); and six are scenarios a second implementation could not run from
their text alone (F-175 to F-180). The answers are #840's coordinator's,
posted on #840. Written against core 0.36, unchanged: no answer needed the
core, and F-164's "an id is never 0" is stated here, not in core
Appendix B.

The chunks were re-cut for this round: these answers are DB, the runtime
(`zenkey::desired`, `config.v1` fed through it, the owner example's
`--desired`) is DC, and the tools are DD. The text, `scenarios.md` and
Appendix B now say so; 0.1's entry keeps the letters it was written with.

Each rule answer is stated in the text and pinned by a fixture case where
one can pin it. No expectation of a case that existed changes: the new
cases were written by hand and passed as written, and a bless from the
model changed none of them. Three inputs are new: every resource of
`declarations.json` and `bindings.json` gives its parameters' types
(`params`, F-170), every reading of `conclusions.json` gives the
deployment's word (`deployment`, F-168: bound for the two cases across a
face, `null` elsewhere), and an `order.json` sample may give a JSON
payload as its text (`text`, F-166). The evidence, counted from the files:
`conformance/declarations.json` (43 cases), `conformance/bindings.json`
(25), `conformance/order.json` (78), `conformance/reports.json` (74),
`conformance/conclusions.json` (35) and `conformance/schema.json` (3), 258
in all, and ten scenario sections.

**Answered: the rules.**
- **F-161, `wrong_self` of a literal or an unbound parameter** (§2.3).
  0.1's words, "the first not bound to `self.system`", made every wrong
  binding of two parameters `wrong_self` too; the fixture's description,
  and its last case, did not. Now `wrong_self` is a parameter bound to the
  other's `self`, the first to `self.service` or the second to
  `self.system`; a literal is `not_self`'s alone, an unbound parameter
  `target_unbound`'s alone. That was the fixture's reading, the model's
  and zk2py's. `bindings.json`: its description, and four cases (a
  literal first, a literal second, the first unbound, and the other's
  `self` beside a literal, which gives both findings).
- **F-162, which version a sample is judged against while an act runs**
  (§2.6, §2.7). §2.6 and the fixture's description judged against the one
  held waiting alone, §2.7 against the running act's and then the held
  one; they differ when the held version has another id, and the first
  let the running act's version be held again over a new epoch's and
  acted on twice in one instance.
  - Now §2.6 says §2.7's: against the version last acted on or adopted,
    which while an act runs is the running act's, then against the one
    held waiting.
  - The model judged against the held one alone whenever one was held
    (`Target::observe`), and now reads §2.7's; zk2py already did.
  - `order.json`: its description, and three cases: the running act's
    version again with a new epoch's held (the same version), an older
    time of the running act's id with another id held (older), and the
    author's own sample of a version held waiting from an archive
    (confirmed).
- **F-163, "never an older version" against step 4** (§2.6). Step 4
  compared a version with the held one alone, so an archive's stale
  `400/a1`, after `500/a1` and then the new epoch's `100/b2` were acted
  on, was another id, acted on: a stale plan applied, which S12 measured
  none of.
  - Now step 4 is widened to the bullet. A target keeps, per id, the
    latest time it acted on or adopted a version of that id in the
    instance; a version older than that, within its id, is older,
    whichever id is held and from whichever path, archive or not; an equal
    one is the same version, never acted on twice; a later one is
    admitted, the new-id rule standing across ids.
  - **Memory:** at least the 16 most recent ids an instance acted on or
    adopted a version of. An id forgotten beyond them is read as new, and
    the text states that residual, accepted at this version.
  - The model kept the held version alone; it now remembers 16 ids
    (`desired::REMEMBERED_IDS`, `Target::remembered`), and a unit test
    shows the seventeenth forgotten. zk2py's reading, the held version
    alone, changes.
  - `order.json`: six cases: the archive's stale `400/a1` after `100/b2`
    (older), the same from the author's own path, the old id's last acted
    version again (the same version), an older one while the new id's act
    runs, an adopted version counting as acted on, and sixteen ids
    remembered.
- **F-164, a stamp's id of 0** (§2.4). Stated: the id is never 0, since a
  zenoh id is never zero. The fixture and both readers already refused
  `100/0`. `reports.json`: one case, `0/0`, a time of 0 beside an id of 0.
- **F-165, an absent member in the report's consistency** (§2.10). Now,
  with `converged` and `desired`, an `in_force` that is absent or not
  `observed` is a finding, and `last_known` counts only when true: the
  fixture's reading, the model's and zk2py's. `reports.json`: one case
  (`last_known: false` with no `observed`, no finding), and case 62's note
  names the absent `in_force`.
- **F-166, an integer past 2^53 in equal documents** (§2.7). RFC 8785
  reads a number as a double, so `{"revision": 9007199254740992}` and
  `{"revision": 9007199254740993}` were one document, and an author's
  re-issue past 2^53 was adopted, never acted on.
  - Now equal is JCS-equal with every number compared by its exact value,
    the value its text denotes: `10`, `10.0` and `1e1` are one document,
    2^53 and 2^53 + 1 two.
  - The model's `Content` hashed `serde_json_canonicalizer`'s bytes,
    which write every number as a double, so it read 2^53 + 1 as 2^53. It
    now hashes a canonical form whose numbers are their exact values
    (`desired::exact_number`), read from the payload's text
    (`Content::of_payload`); a decoded `serde_json` value
    (`Content::of_json`, the fixture's `document`) keeps its integers
    exact and reads a float as its double.
  - zk2py compared JCS bytes where both documents are in RFC 8785's
    domain, and exact values otherwise. That reading agrees on every case
    but one: two documents in that domain whose floats differ in text and
    round to one double, which a new case pins, and which it now reads as
    two.
  - `order.json`: the `text` input, and four cases: 2^53 against
    2^53 + 1 (acted on), `10` against `10.0` and `1e1` as their text (one
    document), two integers past 64 bits as their text (two), and `0.1`
    against `0.10000000000000000555`, one double and two exact values
    (two). Case 46, `10` against `10.0` decoded, stands.
- **F-167, "every number finite" for an integer's text** (§2.10). Now
  finite is defined in this text's words as `telemetry.v1` §2.6 defines
  it, by rounding, so 10^400 written out, `1e400` and
  `1.7976931348623159e308` are not strict JSON, in any member, and the
  largest double's text is.
  - The model's reader already read so, through `serde_json` with
    `float_roundtrip`; it now reads through `telemetry.v1`'s strict reader
    (below), and a unit test pins the edge. zk2py already read so.
  - `reports.json`: four cases as `text` (a revision of 10^400 written
    out, `1e400` and `1.7976931348623159e308` in a member the schema does
    not name, all `json`, and `1.7976931348623158e308` there, read).
    `order.json`: one case, a document with `1e400`, which does not read
    and so compares byte for byte.
- **F-168, the kill switch across a constrained face** (§2.17). Step 1
  said the deployment's word "stands for the descriptor, and step 3",
  and the fixture had no input for that word: a role unbound across a
  face read `unobservable`, `nothing_crossed`, never `unbound`.
  - Now the deployment's word answers step 2 across a face, listed and
    bound, as the descriptor does elsewhere; with no word, nothing stands
    for the descriptor: **unobservable**, `no_descriptor`. A kill switch
    across a face is answered before any report is read.
  - The model's `TargetReading` gains `deployment`, which `premises`
    reads across a face in the descriptor's place. zk2py's reading, step 2
    not read across a face, changes.
  - `conclusions.json`: the `deployment` input, and four cases: a role
    unbound across a face (not asked, `unbound`; in force **no**,
    `unbound`), the same with the kill switch's report crossed, not a
    target, and no word.
- **F-169, the kill switch's report among several roles** (§2.10,
  §2.12). Stated: a report with no `key` belongs to no role a reader asks
  about; it tells whoever reads that member that the role it was put for
  is unbound, and §2.17 answers the kill switch from the descriptor,
  never reading it. No fixture: no reader reads it.
- **F-170, a target parameter's type** (§2.2). A seventh reason,
  `not_string`: a target parameter whose type is not `string`, since a
  system or a service name is a string. It reads the type of a name that
  is a parameter and not a rest one, so a rest parameter (`path`, core
  §2.2) stays `rest_param`'s alone and a name that is no parameter has
  no type, by §2.2's own rule that a check needing a value an earlier one
  found wrong is skipped.
  - The model's `declaration` takes the resource's parameter types, and
    `declaration_of` passes a contract's. zk2py read no reason, as 0.1's
    closed list said; that reading changes.
  - `declarations.json`: `resource.params` on every case, and six cases:
    a `uint` target, a `uint` second of two, a `uint` parameter that is no
    target (`param_not_target` alone), a misspelt name beside a `uint`
    one, a `uint` named twice, and a `uint` target on a stream.

**Also stated: a depth limit** (§2.10, §2.7; found while answering
F-167, decided by #840's coordinator before round 53). 0.1 bounded no
depth, so two readers stopped at two places: the model, through
`serde_json`'s recursion limit, refused a report 128 arrays and objects
deep as `json`, and zk2py read one 300 deep.
- Now a report or a document nests at most 256 arrays and objects deep,
  counted as `telemetry.v1` §2.6 counts it, the top one at depth 1; a
  report with one at depth 257 is `json`, and a document with one does not
  read, so it compares byte for byte (§2.7). `alarms.v1` §2.4's reading
  is unchanged.
- The model reads a report and a document through `telemetry.v1`'s
  strict reader (`desired::strict_json`, `desired::MAX_DEPTH`, 256), so
  `serde_json`'s recursion limit decides nothing; every case that read
  before reads alike.
- `reports.json`: two cases as `text`, a report's object holding 255
  nested arrays (256 deep, read) and 256 (257 deep, `json`).

**Answered: `config.v1`'s** (§2.16, and `config.v1` text 0.4).
- **F-171, once per revision against once per version.** §2.7 governs a
  `config.v1` owner fed through this profile: it acts on each version
  once in an instance, and adopts the same document. To keep the two
  rules from diverging, `config.v1` §2.18 (text 0.4) makes a
  `DesiredConfig`'s author move `revision` whenever `values` change; its
  "once per revision" then reads as §2.7's once per version.
- **F-172, the mapping's codes and pointer.** The codes add `busy` and
  `device-refused` for `failed`; the pointer is `/values/<group>`, then
  `/<parameter>` when the refusal names one, each name escaped as RFC 6901
  says. No fixture reads the mapping: a tool shows `config.v1`'s report as
  `config.v1` §5 says.

**Answered: the record.**
- **F-173, `observed`'s description in the schema** (§3). The schema's
  next revision says "acted on or adopted" in `observed`'s description
  and in the Report's, and `outcome`'s names the last act, which an
  adoption keeps; every keyword is unchanged. Its artifact,
  `sha256:43c95259…`, is published in §3 beside `sha256:8c5ffce3…`, which
  stays the profile's, and `schema.json` pins both (its inline copy of
  0.1's file, published still). `zenkey_model::desired::PUBLISHED` lists
  both. The scenarios' `worker.v1` lists the file by its path, so its
  fingerprint moves; it is published in no history.
- **F-174, `bindings.json`'s count.** 0.1's evidence line said 20; the
  file held 21, and the six files 222. The line is corrected as an
  erratum, and this entry's counts are taken from the files.

**Answered: the scenarios.**
- **F-175, the restarted author's answer** (§7). Expected 2 described the
  first draft's design, a zid reused, which 0.1's review dropped. Now the
  restarted author puts v1's document again under its new instance's
  stamp, another id with the same document, so the target **adopts** it:
  nothing acted on, its report moved to that stamp without `last_known`.
  A new section, §10, shows **confirmation**: the same author instance
  answers after its link heals, while the target, restarted during the
  cut, had read the archive.
- **F-176, the baseline report against the first act** (§5). The test now
  holds the target's act from its start until G has its reply; the
  drive's simulation can hold one. Nothing orders a target's seed after
  its tokens, and the text does not make it so.
- **F-177, `tasking.json` in prose.** It is now given in full, byte for
  byte the file `zenkey/tests/contracts/tasking.json`, as `telemetry.v1`'s
  scenarios give `meter.json`; the file is unchanged.
- **F-178, who sees the target during the cut** (§1). S2, a client of R2,
  subscribes to `Rp` and `Run`, and Expected 3 is read through it; §10
  reads through it too.
- **F-179, a re-mint in one session** (§2.11, §4). A re-mint in its own
  session keeps its zid (core §8.1), and its re-put carries that zid and
  a later time; a restart is a new session with a fresh zid. §4's step 3
  says it re-mints in its own session, and its Expected 3 a later stamp of
  the same id.
- **F-180, §9's questions** (§9). Expected 2 to 4 give all four answers.

**Deliberately not changed.**
- **The core.** F-164's rule is this profile's §2.4, as the coordinator
  decided; core Appendix B does not say it.
- **A revision's range** stays 0 to 2^64−1 (§2.4): F-166's other remedy,
  a bound at 2^53−1, would refuse revisions the adopters may write.
- **Where an unbound role's report sits** in a resource shared by several
  roles (F-169's other remedy): no reader reads it.
- **When a target seeds** (F-176's other remedy, "after its tokens"):
  §2.5 keeps "at its start", and §5's runner holds the act instead.
- **The size of a target's memory beyond 16 ids,** and so its forgetting,
  which no fixture can pin when an implementation may remember more.
- **The scenarios' two contracts,** already given in full; their comment
  lines differ from the reference files', which no fingerprint reads.
- **`alarms.v1`'s reader,** which this profile's §2.10 cites: its own
  depth stays its text's question.

## 0.1 — 2026-10-24: the first text, draft (#840, DA)

`desired.v1` is the profile that three places defined each on its own:
`config.v1` §2.18's `DesiredConfig`, whose CHANGELOG defers to it three
times (its 0.1's and 0.3's entries: drift, and its own words); the
walkthrough's `mission_plan.v1`, with r4 §4.3's executor and spike S12's
measurement; and ZenSight's `zs.desired.v1`, whose v1 implementation
(`zensight-sensor-core/src/desired.rs`, `zensight-desired`) is the target.
Core Appendix D's interim row (`desired | target_param, target`) was all
the core said. This text is written against core 0.36, which publishes the
vocabulary.

**Decided on #840, before the text** (the maintainer's defaults, not
reopened here).
- **What the profile contributes:** point 2, a published vocabulary
  replacing the interim row: `target_param`, one template parameter or a
  list of them, each binding to `self.system` or `self.service` by the
  order R2's binding gives; `target` retired, since the binding already
  says which identity and no contract writes it, and W105 warns of it.
  Point 1, a standard schema, `desired.v1.json`, holding the target's
  `Report`. Point 4 through `uses`. No standard contract: every author's
  interface is its own.
- **A document** is state of its author, keyed by target, one template
  per document type (D22). A target binds its own slice (R2). An optional
  requirement left unbound is the kill switch: no desired state.
- **Version and order:** a document's version is the owner's stamp (S7),
  which every state put already carries; the target orders by it, refuses
  an unstamped sample, and never acts on a version older than one it has
  acted on; a type MAY carry its own `revision`, as `DesiredConfig` does,
  reported beside the stamp; an author's failover is S7's new-id rule,
  and fencing a stale author is `redundancy.v1`'s.
- **Absence and cancellation:** a delete means there is no desired
  document, and the target reverts to its baseline (ZenSight); the
  author's tombstone window MUST cover the longest outage between author
  and target (S3; `link.v1`'s window floors); ending a plan is an act, so
  it is a terminal document of the type, not a delete (E20).
- **Reading:** the target subscribes to its slice, then seeds with S4's
  owner GET at start and whenever the owner's presence appears (§4.2's
  late joiner); with the owner absent, it MAY read an archive's
  last-known (S6), and reports what it acted on as last-known; a periodic
  re-read is a deployment setting, not a rule.
- **Acting:** a target acts on each version once in an instance, whatever
  the outcome, and a restart acts again (`config.v1`'s rule,
  generalised); a document is not a lock, so a later local change does
  not make it converge again, and there is no drift detection at 1.0; the
  target refuses whole, applying nothing, a document that names what it
  needs to reach the bus (`config.v1`'s reach rule, generalised), or one
  its type refuses, and the previous version stays in force.
- **The report** is the standard `Report`, in the target's own state:
  `observed` (the stamp, plus the `revision` when the type carries one),
  `outcome` (converged, refused, failed or reverted), `refusal`, `source`
  (baseline, desired or a call), `last_known` when it came from an
  archive. `config.v1`'s `desired` maps onto it compatibly; its text keeps
  its own words and cites `desired.v1`.
- **The author** writes only on a content diff; after a restart it reads
  its own record, or an archive on its side, never a storage GET of its
  own keys (S4); it deletes a target's document only on positive
  evidence.
- **Not at 1.0:** drift reconciliation, a standard `valid_until`, merge or
  patch documents, an archive's alignment status (core §4.4), and the
  reach-free rule as a lint (the server refuses).

**Stated.**
- **Who implements it** (§2.1), **the declaration** with a closed list of
  reasons (§2.2), **the binding** with a closed list of findings and the
  slice's key (§2.3), **the document and its version** (§2.4),
  **reading** (§2.5), **the order of versions** (§2.6), **acting**,
  the same document adopted without an act (§2.7), **absence, deletes and cancellation** (§2.8), **last-known**
  (§2.9), **the report** with its consistency rules (§2.10), **the
  author** (§2.11), **the kill switch** (§2.12), **population** (§2.13),
  **constrained faces** (§2.14), **the tokenless set and access control**
  (§2.15), **`config.v1` as a target** (§2.16), and **what a tool
  concludes of one target** (§2.17).
- **The standard schema,** `desired.v1.json` (§3), artifact
  `sha256:8c5ffce3…`: `desired.v1.Report`, `desired.v1.Version` and
  `desired.v1.Refusal`, every object open.
- **What a tool may conclude** (§5): has the target converged on its
  author's current document; is its last act refused or failed; is what
  it acted on last-known; is a desired document in force. The changes
  against v1's `@desired` plane (Appendix A), and the adopters' mappings
  (Appendix B).
- **Evidence:** `conformance/declarations.json` (37 cases),
  `conformance/bindings.json` (21; this line said 20 until the erratum of
  0.2, F-174: the six files held 222), `conformance/order.json` (64),
  `conformance/reports.json` (66), `conformance/conclusions.json` (31),
  `conformance/schema.json` (3), each expectation computed twice, by
  `zenkey_model::desired` and by an independent transcription of the
  text, before it was written; nine scenario sections, eight the
  runtime's and one a tool's, over two test contracts, `tasking.v1` and
  `worker.v1`, kept under `zenkey/tests/contracts/`.
  `zenkey_model::desired` runs every fixture.
- **The examples' contracts need no change.** `mission_plan.v1` declares
  `"desired.target_param" = "vehicle"` on `plans/{vehicle}`, and
  `zs.desired.v1` `["host", "service"]` on seven templates, each with
  `freshness.ttl_s = 0`: both read as declared (§2.2), and neither writes
  `desired.target`. No revision is published.

**Decided here, where the record was silent.**
- **Which parameter binds to which.** One target parameter binds to
  `self.system` or `self.service`, the deployment's choice (the
  walkthrough's `{vehicle}` is a system); two bind in the value's order,
  the first to `self.system` and the second to `self.service`, as R2
  lists them and as ZenSight's `["host", "service"]` reads. At most two:
  a target is one service, a system and a service.
- **Every template parameter is a target parameter,** so a target's slice
  is one key and one document per document type. A template with a
  parameter that is no target's would make each target's slice a
  collection, with no rule here for its members' deletes and reverts.
- **One exact author per role.** Two authors' documents for one target
  would contend, and a wildcard binding (core R1) names every implementer
  on a system. A `redundancy.v1` standby holds only its instance token, at
  the same address.
- **The order is core §4.3's,** not zenoh's `Timestamp` order across ids:
  by time within one id, the first value under another id accepted and
  the order restarted there. This is the issue's "S7's new-id rule", and
  what core `scenarios/state.md` §7 requires of a consumer for a new
  epoch whose clock runs behind; zenoh's order, which ZenSight's v1 used
  (`ts <= last`), would ignore that epoch until its clock passed the old
  one's. Within one id the two agree, and agree with core §4.2's merge;
  across ids the case is the core's residual risk, as core 0.26 left it.
  With adoption (on review, below), a new id carrying the documents the
  targets already acted on moves none of them: ZenSight's v1 sensor, given
  a new author's re-put of an unchanged document under a later stamp,
  applied it again; here a failover, like a restart, is quiet.
- **When an act is recorded.** A version is acted on once from the
  moment its act starts, so a re-delivery while it runs is the same
  version. While an act runs, the target holds the latest admitted
  version, the latest replacing an earlier one held, and acts on it next:
  the target converges on the latest, and a version passed over while it
  waited was never acted on, which is not "older".
- **A rejected version is not tried again,** nor the same document put
  again. ZenSight's v1 sensor left a rejected sample's timestamp
  unapplied, so every re-GET re-rejected it; here the act is recorded,
  whatever its outcome, as `config.v1` §2.18 does, and a later version of
  the same document is adopted refused. The next document is the author's
  to make.
- **Refused or failed.** `refused` means nothing of the document applied
  and what was in force stays; `failed` means it applied in part. A target
  that undoes a partial act reports `refused`. After `failed`, no one
  version is wholly in force: `source` is `desired` for a put and
  `baseline` for a delete, `in_force` absent. A revert is never refused;
  one that cannot finish is `failed`.
- **The report carries what is in force beside the last act.** The issue
  listed `observed`, `outcome`, `refusal`, `source` and `last_known`. With
  those alone, after a refusal a reader could not tell which version
  stays in force, which ZenSight's `AppliedConfig` says
  (`desired_timestamp` beside `last_rejected`). So `in_force` is added: the
  version wholly in force, with `source` `desired`.
- **The report carries its document's key,** and none for an unbound
  role. A target may hold several roles, from several authors, possibly
  reported in one templated resource (ZenSight's `applied/{topic}`), and
  roles of two interfaces may share a name: the slice's key is what tells
  them apart, and a tool computes it from the binding (§2.3).
- **A stamp is a string** in zenoh's own form, `<time>/<id>` (uhlc's
  `Display`, which ZenSight's `desired_timestamp` already writes): its
  time is a 64-bit value, beyond what a JSON number carries exactly. Core
  §7.3's subset has no `pattern`, so the form is §2.10's rule, read by a
  reader, not the schema's.
- **The revision** is a top-level member, or protobuf field, named
  `revision`: the two adopters' and `config.v1`'s spelling. It orders
  nothing.
- **Refusal codes.** The profile's own are `undecodable`, `reach`,
  `sensitive` and `invalid`, and `apply` for a failure; a type MAY use its
  own, as `config.v1`'s are, and a reader shows a code it does not know.
  `sensitive` is added beside reach, `config.v1`'s word (on review): a
  document is published to every reader granted it and kept by archives,
  which `config.v1`'s `sensitive` and ZenSight's never-list both refuse. A
  `pointer`, a JSON Pointer, says where, which `config.v1`'s group and
  parameter map onto.
- **A delete reverts only on positive evidence.** A target reverts on a
  delete or a `reply_del`, never on a GET that drew nothing for its key:
  that is how core §4.4's archive drops a key, and an empty reply set is
  no verdict (O5). A delete lost past the window therefore leaves the
  previous document in force (`scenarios.md` §3), which is what the
  window's MUST prevents.
- **Last-known is confirmed by the author.** When the author's answer
  later carries the version the target acted on from an archive, the
  target holds it as the author's and reports so, without acting again.
  When it carries the same document under another stamp, as a restarted
  author's re-put does, the target adopts it, and its report says the
  author's.
- **Equal documents** (the coordinator asked for the rule to be decided
  and pinned). JCS-equal (RFC 8785) for a payload encoded as JSON, read as
  strict JSON; byte-equal for anything else (CBOR, protobuf, raw) and for
  JSON that does not read. Decoding CBOR or protobuf to compare values
  would need the type, and two encoders' bytes of one value can differ;
  byte equality can only cost an act, never skip one. A delete equals a
  delete. `order.json` pins each.
- **Equal to the last act's document, not to the one in force.** A
  refused document put again is then adopted refused, never refused again
  at each restart of its author; a document put again after a call is
  adopted, so that the author's restart never undoes an operator's call;
  and a document equal to an earlier one in force, after a different one
  was refused, is acted on, which is harmless. The comparison needs no
  memory beyond the last act.
- **An adoption moves the report:** `observed` to the adopted version,
  and `in_force` with it when it named the previous one; `outcome`,
  `refusal` and `source` stay; `last_known` says where the adopted version
  came from. A tool comparing the author's current stamp with `observed`
  so finds a target that adopted a re-put converged, never behind.
- **An author's restart deletes again** each key its record holds a
  delete of within its window, so that a target cut off across the delete
  still finds a `reply_del` stamped by the author's present instance; a
  target that had reverted adopts it.
- **Positive evidence for an author's delete** is its own source no
  longer yielding a document for a target it still knows, or the
  deployment's inventory removing the target: ZenSight's author deletes
  only for hosts still in its catalog, after a grace of passes. A target
  absent from presence, or an inventory read that failed, is no such
  evidence. The issue's "the target is gone" is read so.
- **When to seed.** At start, whenever the author's instance token
  appears (never its interface token, which may be tokenless), and, a
  SHOULD, after a loss. Across a constrained face presence does not
  cross, so the deployment's period is the only re-read, and a deployment
  SHOULD give one there.
- **The kill switch's report** is `{"source": "baseline"}`, with no key:
  ZenSight's "disabled must never read as silent", while an unbound role
  has no slice to name.
- **The first report is put before the tokens,** as `health.v1`'s status
  is (core §8.2 lets a profile make it a MUST), so that a GET made when
  the target's instance token appears finds it.
- **The report's consistency** is checked by a reader: eight findings
  (§2.10), each one the state machine never produces. A report that
  breaks one, or does not read, answers nothing (§2.17).
- **`not_yet`** is a fifth token, as `config.v1` §5 has "not yet": a
  target that has not acted on its author's current version yet is
  neither a no nor unobservable.
- **`@state` for large populations** is a SHOULD (§2.13), after
  ZenSight's G7. `zs.desired.v1` declares plain state, which it may keep.

**Decided on review** (2026-10-24, #840's coordinator, after the first
draft of this text).
1. **No zid reuse, and no core change for it.** The first draft had the
   restarted author answer under its record's stamps, keeping its zid, and
   softened core §1.5 to let it. Dropped: the instance id is a continuity
   epoch, and zenoh may refuse or confuse a zid reused within the old
   lease. Instead, a target **adopts without an act** an admitted version
   whose document equals that of the last version it acted on or adopted
   (§2.6 step 5, §2.7), and an author whose instance changes puts each
   document it keeps again from its new instance, before it answers a GET
   with it, as `alarms.v1` §2.9's owner does (§2.11). Its restart moves no
   target to act again (`scenarios.md` §6). "Writes nothing unchanged"
   becomes the author's rule within one instance's life, a SHOULD: a
   re-put now costs messages, never an act. A type that wants the same
   content acted on again carries a `revision`, or changes any member.
2. **Horizons.** A horizon above 0 no longer makes a declaration invalid:
   its re-puts are equal documents, adopted. `freshness.ttl_s = 0` stays a
   SHOULD (D16): an author that hangs leaves its documents in force at
   their targets, which is what desired state is for. A horizon above 0
   means what `freshness.v1` says: the author re-puts, each target adopts
   (a put and a report put per target every ttl/2), a stale document stays
   in force and a tool MAY say it is stale; and across a constrained face
   it is the author's liveness evidence, which a horizon of 0 never is
   (§2.2).
3. **`secret` is `sensitive`,** `config.v1`'s word.
4. **Deletes on positive evidence** as the first draft read #840: the
   author's own source no longer yields the document, or the inventory
   removes the target; never because the target is absent from presence.
5. **Version order** as the first draft had it, core §4.3's rule, with
   adoption making a failover quiet (§2.6).
6. **The rest stands:** the binding order, one author per role,
   `in_force` and `key`, stamps as strings, the eight consistency checks,
   `not_yet`, a revert only on a delete or a `reply_del`, and last-known
   cleared on the author's confirmation.

**Deliberately not stated.**
- **No drift.** A tool that wants to know whether a converged document
  still holds compares it with what the target runs; `source` `call` says
  that a call changed it since. Drift reconciliation is a later minor's.
- **No `valid_until`.** The walkthrough's `Plan.valid_until_ns` is the
  type's own, refused `invalid` once past (§2.7).
- **No merge or patch documents:** a document is put whole.
- **No archive alignment status:** core §4.4's `confirmed` plays no part
  at 1.0.
- **No lint for reach.** The class of a member is the target's knowledge,
  so the server refuses (§2.7); no core lint reads an annotation's value
  either, so §2.2's invalid declaration is a linter's warning.
- **No `history`.** S4's GET at each seed converges; v1's
  AdvancedSubscriber was an accelerator.
- **No core change for the author's restart.** Core 0.36 publishes the
  vocabulary, and nothing else: the restart's re-put is `alarms.v1` §2.9's
  precedent, and adoption is the target's rule (on review, above).
- **No new core rule for ordering.** §2.6 is core §4.3's consumer rule;
  the residual risk of two ids is the core's, unchanged.
- **`config.v1`'s text is unchanged.** Its §2.18 keeps its own words, and
  its report its own type; it cites this profile once its owner reads a
  document through it (#840, DB).
- **No runtime and no tool here.** `zenkey::desired`, the author's writer
  and the target's reader, `config.v1`'s owner fed by it and the owner
  example's `--desired` are #840's DB, and run `scenarios.md` §1–§8;
  `zenctl desired`, the doctor's `desired-*` checks and `check conform`'s
  cases are its DC, and run §9.
