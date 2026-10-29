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
admin space off by default, and §6.1's opening sentence.

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
