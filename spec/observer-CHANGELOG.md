# zk2 observer changelog

Versions of the text of [`observer.md`](observer.md). Each entry records
what changed, what deliberately did not, and why, amendment-style, as
[`CHANGELOG.md`](CHANGELOG.md) does for the core.

**Why a file of its own, beside `core.md`.** `spec/` is flat: the core is
`core.md` beside its `CHANGELOG.md`, its schemas and its evidence. The
observer text is a second text at the same level, so its changelog sits
beside it under its own name, and `CHANGELOG.md` stays the core's, which
every link and `scripts/check-spec-status.sh` already read as such. A
profile has a directory because it carries its own fixtures and
scenarios; this text carries none: its evidence is the core's
`conformance/` and `scenarios/`, and the profiles'.

## 0.4 — 2026-10-30: editorial: inline history taken out of the rules, no rule changed (#935)

Its rules carried their history, as the core's did (#935, epic #921
Track B). Core 0.60 takes it out of both texts at once, and this text is
written against it. **No rule changed.**

**Taken out:**
- **Version and finding tags**: every "(0.N, #n)", "(0.N, F-n)" and
  "(text 0.N, F-n)" in the rules and in Appendix B's rows. A reason a tag
  carried stays, in plain words: §3.3's round trip at the interests
  timeout is "made, and what it establishes is nothing", and §3.7's
  watch session needs a session that never held a subscriber, "one
  undeclared leaves its session exposed".
- **§0's convention for tags.** With no tag left, "A version in
  parentheses …" now says where a rule's history is: this changelog, the
  core's, and Appendix A.
- **"Moved from core §n (core 0.58)"** at the head of §3, §3.10, §4, §5,
  §6, §7.1 to §7.4: each now names the core section it is the tool half
  of. §4.4's two "that sentence moved" clauses are gone; Appendix A keeps
  where each block came from.
- **History told as text.** §4.3's "0.11's second selector" is now "a
  selector such as …", and §5.2's "0.25 read a stalled reader's loss from
  the owner's token" says that the token does not show one. §4.5's
  "(#613)" says that no profile defines an alignment status yet.
- **Notes about one implementation**: §3.3's zk2py follower settings, and
  §3.7's "zk2py's follower does so". The reference's own practice stays.

**Moved to core Appendix B**, under "Measured for observer.md's reads",
each rule keeping its reason in words and a pointer:
- §3.1's hang from 996 tokens, and zenoh-python's 257 of 2,002;
- §3.3's 6,121 of 10,000 at 10 s, and the whole read in 22 s;
- §3.4's `liveliness_query` deny on ingress and on egress;
- §3.5's `transports()` missing the cut in 4 of 10 runs;
- §3.7's round-trip times: with and without history, on the wire keys,
  after a reopen, and a session's first read against a later one;
- §3.7's 450 reads folding none, and the GETs behind one its timeout
  ended;
- §3.8's five reads on a token's own session, and §3.9's two peers.

zenoh-python's missing unbounded handler (§3.1) and its missing accessor
for the interests timeout (§3.3) go to core Appendix B's "One binding's
facts".

**How "no rule changed" was checked**, as for core 0.60: the 39
sentences holding an RFC 2119 keyword, before and after, keyed by
section. 32 are unchanged, three lost a tag only, and four lost a tag
and a measurement or an implementation note, their keyword and condition
kept. This text's tags went from 109 to none, and its sentences over 50
words from 24 to 14.

**Deliberately not changed.**
- **No rule**, section number or label.
- **Appendix A**, which is history: where each block came from.
- **Long sentences.** Splitting them is #935's next part.

## 0.3 — 2026-10-30: zk2py round 83's F-291 to F-294

zk2py round 83 (PR #952) read text 0.2 and core 0.59 cold and found
F-281 to F-290 resolved. Its four findings change no verdict either
implementation gives.

**Changed: rules and wording (round 83).**
- **"Not this profile's" in two spellings** (§1; F-291). 0.2 kept it a
  token of its own "in every medium", and three profiles spell it
  otherwise: `freshness.v1`, `telemetry.v1` and `desired.v1` answer not
  asked with a reason naming the failed premise. §1 now names both
  spellings, each profile's fixtures saying which, and keeps what holds of
  both: never folded into Unobservable, never no. No profile's verdict
  moves, and Appendix B gains the three fixtures.
- **Which medium states a scope's exclusions** (§8.2; F-292). 0.2 said a
  tool states them, not where. Now: in every medium it writes, a field of
  its report or, for a stream of rows with no place for one (NDJSON), its
  standard error. `grammar.md` §1 step 5 says the same. The reference's
  `zenctl echo` now states its scope on standard error in every format.
- **The transient half's step** (§3.2; F-293). 0.2 placed the shape
  "between core §8.2's steps 3 and 4"; the instance token is step 4's
  first token, so the shape is within step 4, after the instance token
  and before the interface tokens. `operations.md` §8 step 6 is reworded
  alike. No verdict moves.
- **§5.3's citation** (F-294): `budget.md` §1–§3, its owner-side §4–§7
  left out, and Appendix B indexes it for §5.3 too.

**Stated: the reference's practice.** The tooling guide's §8.5 records
the reference doctor's 8 MiB memlock floor (spike S9), which round 83 saw
no text state: practice, not a rule.

**Deliberately not changed.**
- **The three profiles' fixtures.** Moving them to the token would move
  their verdict tokens for no reader's gain; §1 reads both.
- **The core version.** Nothing here needs core 0.60.

## 0.2 — 2026-10-30: zk2py round 82's F-281 to F-290, evidence indexed, and three rules given a scenario step (#950)

zk2py round 82 (PR #949) read core 0.57 and 0.58 and text 0.1 cold: the
move lost no rule, and every MUST and SHOULD stands where 0.1 put it.
Its ten findings change no verdict either implementation gives. This text
answers the eight that are this text's, all accepted; F-285 is core
0.59's, written against here, and the scenario steps F-282, F-284 and
F-287 asked for are core 0.59's too, since the scenarios are the core's.

**Evidence cited, and indexed** (Appendix B gains a row for each):
- **F-281, §4.4.** The MUST to compare zids by value cites
  `[F: profiles/timing/conformance/lifespans.json]`, whose case 16 reads
  `meta.zid` `001f` against stamps `1F` and `01f`, each the owner's. 0.1
  said it cited nothing "since this text invents no fixture"; the fixture
  existed, timing.v1's.
- **F-282, §3.2.** The two-reads MUST cites
  `[Sc: operations.md §8; profiles/hostid/scenarios.md §6; profiles/redundancy/scenarios.md §12]`,
  and names its transient half: an owner between core §8.2's steps 3 and
  4 serves a descriptor that lists an interface whose token it has not
  yet declared, and seen in one of two reads a grace apart that shape is
  no fault. `operations.md` §8 step 6 (core 0.59) is that half, the one
  no scenario showed: the token a second after the descriptor, read once
  within that second and twice a grace apart after, no finding.
- **F-283, §3.8, §3.9 and §7.1.** §3.8 and §3.9 cite
  `[Sc: profiles/redundancy/scenarios.md §5]` (a refused read on the
  claim's own session; peers that link), which core Appendix C named for
  them; §7.1's liveness across a face cites
  `profiles/link/scenarios.md` §6 beside `constrained.md` §3; and the
  timing scenarios §4 and §5 index §4.4 beside §8.3, as F-281 notes.
- **F-284, §7.2.** Its citation changed in 0.1, which said each block
  moved "its `[F:]` and `[Sc:]` citations with it": core 0.57's block
  shared `[Sc: operations.md §2, §3, §6]` with the owner's half, which
  core §5.1 keeps with it, and 0.1 cited `[Sc: operations.md §3, §5]`,
  §3 the owner's half and §5 the premise (a refusal is silent). This
  entry records it. `operations.md` §5 step 2 (core 0.59) now checks the
  rule: a tool told that its grants let it call holds a frozen server's
  silence the O3 finding, and one not told holds it unobservable, naming
  the premise. The citation stands.
- **F-287, §8.2.** `grammar.md` §1 step 5 (core 0.59) checks what a tool
  says of its wildcard scope: over `zk2/g/**` it reports `@stream`,
  `@state`, `@op`, `@adv` and `@zk` excluded. 0.1 promoted the rule on
  §1's guard alone, the premise. The citation stands.
- **F-286, §1's not this profile's**, below, cites the three profiles'
  `conclusions.json` that give the token.

**Wording:**
- **F-286, §1.** The profiles' fifth answer has a place in the shape:
  **not this profile's**, where a profile's question's premise does not
  hold (a role carrying none of its policy, an interface it does not
  govern, a resource of a kind it does not read), is an Unestablished answer
  of its own, kept apart from NotAsked and Unobservable in every medium,
  and never Established(no). The profiles' fixtures give it as
  `not_this_profiles`, and both implementations keep the token.
- **F-288, §4.4 and §6.1.** §4.4 stated a tool's attribution of a state
  stamp twice, as its first bullet and as the S1 check's opening; the
  two are one bullet now, the S1 check, which keeps the first's
  "unattributable, never foreign" and its origin, and the zid comparison
  follows it. §6.1's opening restates core §6's sentence whole: the
  runtime does not fence, but for an interface whose contract uses
  `redundancy.v1`, whose holders step down as its §2.9 says.
- **F-289, §0 and §2.** §2's "(core R7, §7.1)" and "(core S4–S6, §4)"
  read as the core's §7.1 and §4; they meant this text's, and read
  "(core R7; §7.1)" and "(core S4–S6; §4)". §0 states the rule they
  follow: a list after `core` is the core's throughout, and a semicolon
  ends it. Every other such list here (`core §4.2, §5.1, §3.2`, `core
  §8.4, §9.6`, `core §1.5, §8.1`, `core §4.1, §4.3`) names core sections
  only, so none changes.
- **F-290, §7.4.** The SHOULD names the limit core §7.4 sets, the pool
  plus 2 MiB, but the pool is a publisher's, which a tool at its start
  does not know. The check reports what the limit admits: the largest
  pool the soft limit covers, the limit less 2 MiB, or, below 2 MiB,
  that it admits none; an unlimited limit admits any pool, and one the
  tool cannot read leaves the check unobservable. zk2py's tools said so
  already (`live.memlock_note`); the reference's doctor says so since
  this text (`shm-memlock-low`).

**The reference** (#950): `zenctl timing` and `zenctl telemetry`, given
a selector, report what it cannot reach in a `scope` field of their
report (the verbatim chunks it names none of, and the timed streams or
metric resources of the owners in scope it misses, each with its key
expression), in a note, and on stderr through the raw observers' line
(#924), and an empty scope's reason names the first missed. Round 82
measured `zenctl timing` under `zk2/lab/beat/**` watching `stream/cmd`
alone and saying nothing of `@stream/frame`. `zenctl alarms` needs none:
every `alarms.v1` key is plain state, so the chunks a wildcard excludes
hold no alarm. `zenctl arbitration` offers no wildcard scope: it
subscribes to each binding entry's stream by its contract's key
expression.

**Deliberately not changed.**
- **No rule.** Every MUST and SHOULD binds whom 0.1's did, as 0.1's did;
  §3.2's transient half and §7.4's report spell out what the rules
  already meant, and §1's not this profile's names a token the profiles
  published before this text.
- **No fixture.** Each citation added names a fixture or a scenario that
  existed, but for the three scenario steps, which are core 0.59's.
- **The guidance kept in the tooling guide** stays there: none of 0.1's
  "Deliberately not promoted" gained evidence here.
- **Every profile's "Written against"** stays as it is.

## 0.1 — 2026-10-29: the reader's discipline out of core 0.57, and the tooling guide's judgement shape and O-rules (#925)

The first text, written against core 0.58, status `draft` until a cold
read by the second implementation (#925's barrier round) finds nothing
lost. It is normative for the `tool` role, and for any participant where
it reads presence, state or the admin space and concludes from what it
read (§0).

**Moved from the core, no rule changed** (core 0.58 lists each block's old
and new home). §3 is core §8.1's "Reading presence" and §8.3's count; §4
core §4.2's seed, admin-space and S1 checks, §3.3's tool sentences and
§4.4's alignment as a tool sees it; §5 core §2.4's and §2.7's readings and
conclusions; §6 core §6's split-brain diagnosis and the standby mark's
reading; §7 the tool's halves of R3 and R7, a tool's `self.system`, a tool
judging O3, decoding and honest rendering, and the SHM check. Each block
is word for word core 0.57's, with its `[F:]` and `[Sc:]` citations; its
references are rewritten for this text (§0: a bare § is this text's, a
core section is `core §n`, and a version in parentheses is the core
amendment that made the rule). Two blocks a core block shared with an
owner or a deployment restate the core's half and cite it: §4.2's
admin space off by default, and §6.1's opening sentence. Where only a
sentence or a table cell moved, it is a bullet of its own here, its words
kept and the subject it lost named: §4.4's two `meta` sentences, §7.1's
R3 and R7 clauses, §7.3's "What a generic tool decodes" (core §7.1's
"Generic tools" column and its plugin), and §7.4's SHOULD, which names
the limit core §7.4 sets.

**Promoted from [`docs/zk2/tooling-guide.md`](../docs/zk2/tooling-guide.md),
each with its evidence:**
- §1, the judgement shape (guide §1): Established yes and no, NotAsked,
  Unobservable with its reason; the three media (guide §3); the two
  unestablished kinds kept apart; not asked is not answered no (guide
  O4); an empty scope unobservable, never clean.
  `[F: budget/population.json, budget/rate.json]`
  `[Sc: presence.md §6; profiles/desired/scenarios.md §9]`
- §2, silence is never a verdict (guide §2), its three conditions and its
  instances, each pointing where it is normative.
  `[Sc: operations.md §5; presence.md §6]` `[F: budget/population.json]`
- §8.1, a key that is not zk2 is a fact, not an error (guide O1).
  `[F: keys.json]` `[Sc: state.md §11]`
- §8.2, a wildcard scope is not total coverage, and everything is three
  selectors (guide O5, which said #925 would move it here).
  `[Sc: grammar.md §1, §2]`
- §8.3, a tool that reports a timestamp names who stamped it (guide O7).
  `[Sc: state.md §1; profiles/timing/scenarios.md §4, §5]`

**Changed while promoting.** The guide's wording is kept, but for:
- §1's "Not asked is not answered no" says "in each of the three media",
  which the guide's O4 spelled out medium by medium;
- §8.3's router bullet adds the stamper core 0.55 named, a reader's own
  session whose HLC is enabled (core §4.1), which the guide predates;
- §8.3's zid comparison cites §4.4, where the rule now is, for the
  guide's "core §3.3, 0.11".

**Deliberately not promoted: guidance for want of evidence.** No fixture or
scenario checks these, and this text invents none; they stay in the guide:
- the polarity of a vocabulary (guide §1), and the exit codes 0, 1 and 2 a
  judging tool projects the shape onto, with zenctl's spelling;
- O1's report shape (a partial report, the fields below the failure
  absent, `Unresolved::NotZk2`);
- O2, classifying by degrading through the identity rungs;
- O3, not guessing another deployment's namespace;
- O4's spelling in each medium (the reserved mark, an absent field, a
  note naming how to ask);
- O5's `scopes` field and a recording's header, and zenctl's `--explicit`;
- O6, a bounded observer reporting what it dropped, apart, and R6's
  discards as a fourth count;
- O7's "never pool measurements from different stampers";
- the guide's §4 (a tool writes only through operations, and a mock owner
  is an owner) and §5 (recording and replay), which restate P3 and the
  reference's `.zrec`.

**Deliberately not changed.** No fixture, scenario or core rule. Two
moved MUSTs cited no evidence in core 0.57, and cite none here either,
since this text invents no fixture: §3.2's two reads a grace apart for
every fault read from presence's shape (its split-brain case is §6.1's,
`[Sc: operations.md §8]`), and §4.4's zid comparison by value. Every
profile's "Written against" stays.
