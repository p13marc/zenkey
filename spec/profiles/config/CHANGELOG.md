# `config.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.7 — 2026-10-30: editorial: round 86's F-329 to F-331, no rule changed (#935)

zk2py round 86 (PR #959) read 0.6 cold, beside core 0.62 and
`observer.md` 0.6. It found that 0.6 changed no rule, and filed three
findings against this text, F-329 to F-331, all hard to read. This is
#935's fourth part. Each rewrite is the finding's, and keeps its
keyword, condition, scope and evidence.

**Editorial: round 86's hard-to-read findings.**
- **F-329, §2.3's "Two keys, one revision".** "Never a pair" and "pairs
  them" named two things, and "both" and "them" had two antecedents. "The
  status is the view's projection" read as always true, since §2.19 asks
  it of every status. It is three sentences now. `instance` and
  `revision` never name one version of the two documents. A reader that
  reads the view and the status takes them together only when the status
  it read is the projection of the view it read. Otherwise it takes each
  by itself.
- **F-330, §2.13, persist's first refusal.** "So" stood for the refusal
  the item opens with, "its request's type included" read as part of the
  refusal, and "(below)" pointed past three bullets. The item is two
  sub-items now, each of two sentences:
  - an owner with no layer does not expose `persist`, and the core
    refuses the call before any handler runs, so before it checks the
    request's type ("No persisted layer", below);
  - a layer that cannot be written now is refused the same way by the
    handler, before its other refusals and the request's type, so the
    refusal does not depend on whether the layer was ever there.
- **F-331, §2.18's "A delete".** "A refused document's aside" left its
  noun out. Each parameter named by a document the owner acted on goes
  back to its startup value, and a sentence of its own says that a
  document it refused counts for none: it applied nothing of it. That is
  the reading the finding took, and the only one that leaves no
  converged value standing after the delete. `desired.v1` §2.16 had the
  same phrase, and its text 0.9 says it the same way.

**How "no rule changed" was checked**, as for 0.6. This text's 39
keyword sentences are unchanged, and so are the scenarios. The three
rewrites hold no keyword. They were diffed sentence by sentence and read
beside their old text. No fixture moved: `validation.json` pins the
order of F-330's refusals.

**The figures.** This text's mean went from 18.08 words to 17.95.
`spec/.prose-baseline.json` is lowered to it.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Written against core 0.28**, still.

## 0.6 — 2026-10-30: editorial: inline history taken out of the rules and long sentences split, no rule changed (#935)

The rules carried their own history, as the core's did before core 0.60
(#935, epic #921 Track B). A rule named the version that made it and the
finding that caused it, as in "(text 0.3, F-114)". This changelog and the
core's already hold that history. So the text now states each rule and
cites its evidence, and nothing else. This is #935's third part, beside
`freshness.v1`, `health.v1`, `hostid.v1` and `link.v1`.

**Editorial: inline history taken out of the rules, no rule changed.**
- **This text's versions.** "(text 0.3)" is gone from §2.3, §2.9, §2.12,
  §2.14 and §2.18. So are "(text 0.3, F-115)" in §2.5, "(text 0.3,
  F-114)" and "(text 0.4, F-171)" in §2.18, and "(text 0.5)" in §2.18
  and §5's last row.
- **A reason kept.** §2.18 opened with "(config.v1 text 0.4: an owner fed
  through `desired.v1` acts as its §2.7 says, below; text 0.5, F-182:
  …)". Its reason is a sentence of its own: "An owner fed through
  `desired.v1` acts as its §2.7 says (below)." F-182's note was about the
  tag itself, and went with it.
- **Another tag in §2.18.** "(config.v1 text 0.4, F-172)" is gone from
  the report's mapping, and "(its text 0.3)" from the citation of
  `desired.v1` §2.16.
- **The core's versions.** "(core §5.1, 0.28)" in §2.16 and "(core §2.7,
  0.28)" in §2.17 cite the section alone. "(0.28)" is gone from §2.13's
  sixth refusal.
- **§0's references.** "(the binding, #623)" is "(the binding)". The
  references to the architecture are listed in another order, "its §4.8
  and §4.10, its §0.2 and its decisions D11 and D12". The check reads a
  `0.N` before a comma as a version, and "§0.2," was one.
- **Appendix B.** "(text 0.3; 0.2 said review)" is gone from the fourth
  kind.
- **The scenarios.** Their opening names the reference's tests without
  "(#768, CB)" and "(#768, CC)".

**The long sentences, split.** The five over 50 words:
- **§2.5, the core's refusals,** are a list of four, each with its
  refusal.
- **§2.5, "Why this order",** is a list of five, in the same order.
- **§2.18, "Fed through `desired.v1`",** is three sentences: whom it
  binds, the act on each version, and the adoption.
- **§2.18, "A delete",** is three sentences: the revert, what it puts
  back, and how.
- **Appendix B's fourth kind.** The parenthesised reason follows a colon,
  and "So a fourth kind is a new major" is a sentence.

The MUST that follows §2.5's list said "the first of these", which would now
read as the list above it. It says "the first of the checks below", the
table it always meant. The rest hold no keyword, and keep their
conditions and scope.

**Nothing moved to an appendix.** No rule held a measurement.

**How "no rule changed" was checked**, as for core 0.60 and 0.61.
- **The method.** Every sentence holding MUST, MUST NOT, SHOULD, SHOULD
  NOT, MAY or RECOMMENDED was extracted before and after. Each was keyed
  by section, and the two sets diffed.
- **This text.** Both sets hold 39. 38 are unchanged: every tag sat in
  a sentence with no keyword, or in a run-in label. §2.5's MUST names
  the checks below, as said above.
- **The scenarios.** Their 76 such sentences and Expected lines are
  unchanged.
- **The rest.** The rewritten sentences hold no keyword. They were read
  beside the others. No fixture moved.

**The figures.**
- **This text:** tags from 19 to none, sentences over 50 words from 5 to
  none, and a mean from 18.74 words to 18.08.
- **`scenarios.md`:** a mean from 11.19 to 11.16.

`spec/.prose-baseline.json` is lowered to them.

**Deliberately not changed.**
- **No rule**, and no section number, label or anchor.
- **Appendix A**, which is history.
- **Written against core 0.28**, still, and the header's "uses
  `freshness.v1`, text 0.3".

## 0.5 — 2026-10-24: a delete fed through `desired.v1` (#840, DC; #609)

`desired.v1`'s runtime (#840, DC: `zenkey::desired`, whose
`ServiceBuilder::desired_config` feeds an owner through `desired.v1`'s
reading) settled what 0.4 left to it: what a delete reverts an owner to.
`desired.v1` §2.16 (its text 0.3) states the rule; this text says what
the owner's view and status carry after it. zk2py's round 53 (PR #845)
read 0.4 cold, and filed F-182. Still written against core 0.28. Every
fixture is unchanged: a delete is `desired.v1`'s `order.json`'s, and the
revert is the runtime's test (`zenkey/tests/profile_desired.rs`,
`config_v1_as_a_target`).

**Changed.**
- **A delete** (§2.18). An owner fed through `desired.v1` reverts, on a
  delete it acts on, each parameter a document it acted on in this
  instance named, a refused document's aside, to its startup value (the
  persisted layer's, else the startup file's or the default, §2.12),
  through one `set` without a window per group, as converging does. Its
  view and its status carry no `desired` after it: no document is bound.
  §5's "not asked" names the case.
  - **Why the startup value.** The values a document converged are
    runtime changes (§2.12), which a restart rolls back to exactly these:
    a delete is the document's end, not the process's, so it undoes what
    the documents did and nothing else. A value an operator's `set`
    changed that no document named is not the delete's to undo, and a
    persisted value is a startup value, so it stays.
  - **Why no `desired` after it.** `DesiredReport` reports a document;
    with none bound, the owner converges no desired state, which is what
    an absent member says. Keeping the last document's report would show
    a revision as converged whose values were just reverted. `desired.v1`'s
    Report, where the owner's application puts one, says `reverted`.

**Answered.**
- **F-182, "(text 0.4)" read as `desired.v1`'s version** (§2.18). The
  marker after the link to `desired.v1` was this text's, which reads as
  the linked profile's. It now follows "config.v1 text 0.4", and so does
  the one on the mapping (F-172). A wording fix: no rule changed by it.

**Deliberately not changed.**
- **`DesiredReport`'s outcomes.** No `reverted` is added: the type is the
  standard contract's, and a new outcome would be a new revision of it for
  a fact an absent member already states.
- **A revert is never refused.** No startup value is a reach, contract or
  sensitive one a document could name, since a refused document's
  parameters are not reverted; a revert whose `set` is refused is
  `failed` in `desired.v1`'s words, the groups before it restored.

## 0.4 — 2026-10-24: a document fed through `desired.v1` (#840, DB; #609)

`desired.v1` exists now (its text 0.2), and states this profile's §2.18
as the rules of one document type, `DesiredConfig` (its §2.16). zk2py's
round 52, reading `desired.v1` 0.1 cold, found the two texts disagreeing
in one case and leaving the mapping half said (F-171, F-172); the answers
are #840's coordinator's. Still written against core 0.28. Every fixture
is unchanged: no rule here needs one, the author's being shown by
`scenarios.md` §12, whose every document moves its revision, and the
owner's by `desired.v1`'s `order.json`.

**Answered.**
- **F-171, once per revision against once per version** (§2.18).
  `desired.v1` §2.7 tells two documents apart by their content, this text
  by their revision; they disagreed when an author changed `values` and
  kept `revision`.
  - Now an owner fed through `desired.v1` acts as its §2.7 says: on each
    version once in an instance, adopting without an act a version whose
    document equals the last one it acted on or adopted.
  - A `DesiredConfig`'s author MUST move `revision` whenever `values`
    change, so that the two rules never diverge: "once per revision" then
    reads as `desired.v1`'s once per version, and a revision put again
    unchanged is adopted. A document that breaks the author's rule is
    acted on, as `desired.v1` §2.7 says.
- **F-172, the mapping onto `desired.v1`'s Report** (§2.18). The pointer
  of a refusal is `/values/<group>`, then `/<parameter>` when the refusal
  names one, each name escaped as RFC 6901 says; `desired.v1` §2.16 lists
  `busy` and `device-refused`, `failed`'s codes, beside the refusals'.

**Deliberately not changed.**
- **What a delete reverts an owner to,** which `desired.v1` §2.16 leaves
  to this text: the runtime that feeds an owner through `desired.v1`
  (#840's DC) is where it will be settled.
- **The report's own type.** `DesiredReport` keeps its members and carries
  no stamp; `desired.v1` §5 does not read it, and §5 here answers it.
- **The order of refusals and the codes,** unchanged since 0.1.

## 0.3 — 2026-10-18: round 29's two findings, and the four points 0.2 left open (#768, CC; #609)

zk2py's round 29 read 0.2 cold and passed every fixture (1240 of 1240)
and every scenario section, §1 to §14, against its own owner and against
the Rust owner example's `--config`; the live runner passed 423 of 423,
and the order of refusals matched the Rust owner's on all 37 probes. It
filed two findings, F-114 and F-115. 0.2 also left four points to the
maintainer. Each is decided here the way the text's rules point, and one
that is better left to the owner says so. Still written against core 0.28.

The evidence is now `validation.json`, 114 cases, and `lifecycle.json`, 27
cases. Seven `validation.json` cases were written by hand and pass as
written; the new `lifecycle.json` case was blessed from the model (its
reason: a new case) and read step by step: the first `set` with key `k2`
applied at revision 1 with its event; the same request with `"dry_run":
false` answered that first answer with no act and no event; the same
with `"dry_run": true` refused `invalid_request` on `/idempotency_key`.
No case that existed changes.

**Answered: the two findings.**
- **F-114, a bound desired document after a restart** (§2.18, §2.12). The
  binding is the deployment's, and survives a restart; the values the
  document converged were runtime changes, which the restart rolled back.
  So the new instance acts on the document bound to it as the first did,
  converging the same revision again, and its view and its status carry no
  `desired` until it has acted: a new instance knows nothing of the old
  one's report. §5's "not asked" for no `desired` member now names that
  window. That is zk2py's reading.
  - **The reference already behaved so** for an application that hands the
    document to the new instance (`Configuration::set_desired`, an instance
    acting on each revision once). The owner example did not: its binding
    is its standard input, and `config restart` dropped it. It now carries
    the document to the new instance (`Configuration::desired_document`,
    new), and `zenkey/tests/owner_example.rs` restarts after a convergence
    and sees the same revision converged again, at revision 1 of the new
    instance.
  - No fixture: converging is not a step of `lifecycle.json`, and a
    restart's binding is a deployment's; the owner example's test pins it.
- **F-115, a request member that JSON Schema reads as an integer and the
  core does not** (§2.5, "A request outside its type"). `confirm_s` or
  `expected_revision` written `30.0` or `3e1`, or beyond the unsigned
  64-bit range, is outside the request's type, refused `invalid_request`
  before row 1, in the bytewise order of the member names. A value in
  `values` stays as 0.2 read it: `2.0` is a `ParamValue` row 11 refuses
  against its parameter's kind, because only row 11 knows the kind, while
  each of the request's own members has one type. Both implementations
  already refused these at decode, which the text now says.
  - A member of the wrong JSON type (a `dry_run` that is not a boolean, an
    `answer` other than `view` or `status`) is named in the same list.
  - **A control request's members in bytewise order** (§2.10 said "as §2.5
    says of a set"): `answer`, then `confirm_s`, then `token`. The model
    checked `token` before `confirm_s`, which no case had distinguished; it
    now takes them in the bytewise order. `zenkey_model::config` gains
    `decode_set`, `decode_control`, `validate_set_json` and
    `validate_control_json`, the request's own type checked from the body
    as it arrived, and the fixture runner reads every case through them.
  - `validation.json`: seven cases. `confirm_s` written `30.0` before an
    act in flight; `expected_revision` beyond 64 bits before no such group;
    `expected_revision` `4.0` before a `values` fraction; a `dry_run` of
    `"yes"` before row 5; an extend with `confirm_s` 0 and an empty token
    (`confirm_s` first); a confirm with an unknown `answer` and an empty
    token (`answer` first); an extend with `confirm_s` written `60.0`
    before an act in flight.

**Decided: the four points 0.2 left open.**
- **A hot group left part applied** (§2.9). The refusal is still
  `device-refused`, starting no change, minting no token and publishing no
  event: the profile's changes are the ones a `set` started. An owner whose
  device can be left part applied restores what was applied before it
  answers, once, as an undo restores; a restore that succeeds is no act.
  One that fails leaves the read-back the truth, moves the revision by one
  (the running values moved, §2.3), and SHOULD be reported in the owner's
  own interfaces (`health.v1`). **How** it restores is deliberately the
  owner's, as an undo's means are. The reference's device contract is
  "whole or not at all", so nothing in it changes.
- **A re-mint while `config.v1` runs** (§2.12, §2.3, core §1.5). A re-mint
  is a new instance, and a new instance starts from its startup values. So
  an owner re-mints while `config.v1` runs only with its device running
  them: nothing pending, every runtime change of the instance undone or
  persisted (a persisted value is a startup value). The reference refuses
  the re-mint, the simplest owner that keeps the rule. A ROS 2 bridge that
  re-mints when a node's parameters change (Appendix B) keeps it the same
  way.
- **A converged revision after a later `set`** (§2.18). Each revision is
  acted on once in an instance, whatever its outcome: a later `set` that
  changes a value the document names does not make it converge again, and
  `converged` now says what was true when the owner acted ("was the running
  value"). A desired document is not a lock on the configuration; a tool
  that wants to know whether one still holds compares it with the view. Both
  implementations already acted once per revision. `desired.v1` may add
  drift; this text does not.
- **"An equal request"** (§2.14). Equal as decoded: a member at its default
  is the member absent, so `"dry_run": false` and no `dry_run` are one
  request, as the model read it; every other member, and each value of
  `values`, compares as the JSON value it is. zk2py compared the members as
  written, and changes. `lifecycle.json`: one case (above).

**Corrected.**
- **Appendix B's fourth kind.** 0.2 said a `oneOf` branch added to
  `Parameter` is a review; core §9.8 classifies it breaking (spike S7: a new
  writer sends the new variant, an old reader refuses it). A fourth kind is
  a new major of the contract. Found while CC added the doctor's checks,
  whose closed list of ids hit the same rule in `zenwatch.v1`.

**Deliberately not stated.**
- **Whether a re-mint publishes anything for the old instance's pending
  change.** §2.12's last bullet already lets an owner that keeps a record
  publish what it knows as a change of the new instance; a re-mint is no
  different.
- **Drift of a converged document.** No outcome reports that a later `set`
  changed a value a document names: that is `desired.v1`'s, when it exists.

## 0.2 — 2026-10-18: the cold reader's findings, and what the runtime settled (#768, CB; #609)

Two readings of 0.1 met here. The spec-only implementation read it cold
(zk2py round 28, PR #774): every fixture, 291 of 291, and every scenario
section, §1 to §14, passed, and it filed seven findings, F-107 to F-113.
The runtime (`zenkey::config_v1`, #768's CB) wraps `zenkey_model::config`'s
owner over an application's device, and runs §1 to §12 on in-process
routers. Building it found three more places where 0.1 left a reader to
infer. Written against core 0.28, which answers the four core questions
CA raised (`../../CHANGELOG.md`).

Each answer is stated in the text, and pinned by a fixture case where a
fixture can pin it. Two answers change the model, F-107's order and
F-111's persist, and no case that existed before changes: the new
`validation.json` cases were written by hand and pass as written, and the
two new `lifecycle.json` cases were blessed from the model and read step by
step (below). The evidence is now `validation.json`, 107 cases, and
`lifecycle.json`, 26 cases and 131 steps; the other three files are
unchanged.

**Answered: the seven findings.**
- **F-107, a value outside `ParamValue`** (§2.5). `SetRequest.values` is a
  map to `ParamValue`, a boolean, an integer or a string. A decode that
  holds the schema refuses `null`, a fraction, an array or an object
  before row 1, while 0.1 listed `null` under row 11. An owner cannot
  choose: one whose runtime checks the request against the schema has
  refused it before any handler. So every owner refuses such a value
  before row 1, as outside the request's type. `2.0`, `2e0` and an integer
  beyond 64 bits are integers to JSON Schema, so of the request's type,
  and row 11 refuses them against the parameter's kind, as 0.1 said.
  - **Several faults at once** are taken in the bytewise order of the
    member names, `values` last, and in `values` in the bytewise order of
    the parameter names. That is zk2py's guess for the order among
    members, now stated. The model's order was `confirm_s`, `token`,
    `idempotency_key`, so the two orders differed only for a request with
    both a bad token and a bad key, which no case had. It changes to the
    bytewise one.
  - It overturns zk2py's guess for the values themselves, which left
    every value to row 11.
  - `validation.json`: eight cases. `null` before row 3, a fraction before
    row 1, an array before row 5, an object before row 4, `2.0` after row
    4, an integer beyond 64 bits after row 3, `idempotency_key` before
    `token` and `values`, and the first bad value by name.
  - The fixture's own description says where the type ends.
- **F-108, an empty change** (§2.5, §2.3, §2.11). 0.1 said an empty
  `values` "applies nothing to the device" and that its revision is "as
  any other's", while the revision counted "acts that moved the running
  values". The model, and zk2py's guess, make its apply an act with no
  values: the device is called, the revision moves, a token is minted, and
  the event carries no edits. Under a window, its undo has nothing to
  restore, so no act is made, as §2.11 says of any change with nothing
  applied.
  - The text now says so, and §2.3 counts "the owner's acts on its running
    values", an empty apply included.
  - Rejected: no act at all. A revision that moved without an act would
    break §2.3's "at nothing else", and one that did not move would break
    "as any other's".
  - `lifecycle.json`: a new case. An empty hot change without a window, one
    under a window then cancelled with no act, and an empty reach change
    applied at its `apply_at`, then confirmed.
- **F-109, a value not read back** (§2.3, §2.11). Its `source` is absent
  too: both are unknown. `lifecycle.json`'s case 4 already pinned it, and
  zk2py read it from there.
- **F-110, the edge of the rate's hour** (§2.17). An event published
  exactly an hour before no longer counts. That is core §2.7's span: "two
  instants exactly one span apart are not within one span". Case 19
  already pinned it, and zk2py read it from there.
- **F-111, persist under a pending change** (§2.13). 0.1 accepted it, as
  `validation.json` does, and said each parameter the last change set then
  reads `source: overlay`.
  - **A bug in the model, found by the finding.** The model wrote the
    *running* values of the parameters the last change set. When a pending
    change had set one of them since, it persisted the pending change's
    value, never confirmed: a restart would bring back a change §2.12 makes
    a rollback. It now writes the values the last change set, as it set
    them, which is what "writes the last change" says.
  - **After it**, as zk2py guessed:
    - a parameter that an applied group of the pending change holds keeps
      its value and source, and shows the value written as `startup`;
    - undone, it returns to its value before the change at `source:
      overlay` when that is the value written, not at the stale source
      §2.11 would otherwise restore;
    - a parameter only a reach group not yet applied set still runs the
      value written, and reads `overlay` at once.
  - `lifecycle.json`: a new case, with both branches and a restart after
    each.
- **F-112, pairing the view and the status** (§2.3). The revision counts
  acts, not versions of the documents. An arm, a join, an extend, a
  confirm, a cancel with nothing applied, a persist and a desired report
  each re-put both at the same revision. So `instance` and `revision` name
  the running values, never a pair.
  - A reader pairs the two only when the status is the view's projection,
    and otherwise takes each by itself, judged by its own freshness. That
    is zk2py's reader.
  - An owner SHOULD put the view first, then the status. The reference
    and zk2py both do.
  - No fixture: pairing is a reader's, and `ConfigStatus::of` already
    pins the projection.
- **F-113, a starting reach change its device cannot apply in time**
  (§2.8, §2.9). Row 13 refuses a join only, and 0.1 required no more of the
  floor than 1 s.
  - Now an owner MUST keep its floor above the longest delay its device
    states for a reach `apply_at`, so every change it starts can apply
    before its deadline. Row 13 stays a join's: a join keeps a deadline
    that may have come closer than the floor.
  - The model already refused to start an owner whose `apply_after` is not
    below its floor, and zk2py refuses to construct one: both are the
    rule.
  - Rejected: a row refusing the starting `set`. It would refuse every
    reach change of such an owner, a fault of its configuration, at call
    time, one caller at a time.
  - No fixture: an owner's construction is no call.

**Stated: what the runtime settled.**
- **A persisted layer that cannot be written now is refused before the
  request's type** (§2.13, row 1). 0.1 said "the same way, first of the
  handler's refusals", while §2.10 says a request outside its type is
  refused "first". `validation.json` pins "no persisted layer" before the
  request's type, and the model checks the layer first. "The same way"
  now says so, and a caller gets one refusal whether the layer is absent
  or unwritable.
- **The rate holds across a restart** (§2.17, a line in §2.12). Core 0.28
  states it for every rated event (§2.7). `events/change` is one member per
  service, so a restarted owner counts its earlier instances' events, from
  a record of their instants that it keeps across the restart, as it keeps
  its persisted layer. The reference takes the record from the application
  (`Startup::events`, filled from `Configuration::published`).
- **Converging waits for an act in flight too, and claims no actor**
  (§2.18). One act at a time (§2.7) already holds a converging `set` back
  while another act is in flight, as a pending change does. A converging
  `set` is the owner's, not a call, so §2.17's "absent when it carried
  none" leaves its event without `actor` and `request_id`.
- **The core's words** (0.28): `forbidden` is core §5.2's refusal by a rule
  of the owner's own (§2.13, row 6), and an owner may answer `unavailable`
  from its handler for one member of an operation it exposes, which is
  §2.16's allowlist, group by group.

**Deliberately not stated.**
- **A hot group's device left part-applied** (§2.9: "an owner whose device
  can be left with part of a group applied reads the device back"). The
  model reads a refused hot apply as nothing applied: no change, no token,
  no event, as §2.9 says of a refusal. What the view shows, and whether a
  change starts, when a hot apply leaves part of its group applied is not
  settled. The reference's device contract is "whole or not at all", and
  the question goes to the maintainer.
- **A re-mint while the configuration runs.** §2.3 restarts the revision
  in each instance, and §2.12 rolls a restart back by construction, since
  the process starts again. A re-mint within one running process (core
  §1.5) would restart the revision while the device still runs the old
  instance's values. The text does not say whether an owner may re-mint
  so. The reference refuses it while `config.v1` runs, and the question
  goes to the maintainer.
- **A converged document after a later `set`.** "It tries a failed
  document again when its revision changes" leaves open whether an owner
  converges a converged revision again when a later `set` changes a value
  it names. The reference and zk2py both act on each revision once,
  whatever its outcome. The text keeps its words until `desired.v1`
  decides.
- **"An equal request"** (§2.14). zk2py compares the members as written,
  so `dry_run` absent and `dry_run: false` differ. The model compares the
  decoded request, where they are equal. Neither reading changes any
  fixture, and no finding raised it. It is left for the next round, named
  here so that it is not lost.
- **`busy`'s token in a detail.** Core 0.28 keeps a detail on `app` alone,
  so the token stays in the message for a person, and in the view and the
  status for a program, as 0.1 decided.
- **The scenarios.** Unchanged: §1 to §12 run as `zenkey`'s
  `tests/profile_config.rs`, one test per section. §10's 300 s are the
  owner's clock moved, which is what the step observes.

## 0.1 — 2026-10-18: the first text, draft (#768, CA)

`config.v1` is the fourth profile of #613, after `hostid.v1`,
`freshness.v1`, which it uses, and `health.v1`. Architecture r4 §0.2 says
it takes zenoh-modem's semantics whole, and #623's binding
(`examples/zk2/zenoh-modem/config.v1-binding.md`) sketched its contract.
This text gives it rules, its standard contract, and its evidence, and is
written against core 0.27. The maintainer's decisions on #768 are stated
here as rules.

**Stated.**
- **The contract** (§3): [`config.v1.toml`](config.v1.toml) with its JSON
  Schema types in [`schemas/config.json`](schemas/config.json), revision
  1.0, `sha256:727ff009…`, published in
  [`../.history/config.v1/`](../.history/config.v1/). `state/view` and
  `state/status`, both `freshness.ttl_s = 60`; `@op/set/{group}` (16
  groups), `confirm`, `cancel`, `extend` and an optional `persist`; the
  event `change`, `burst(600/h)`, `7d`. No operation is idempotent, and
  none fans out. `SetReply` and `ControlReply` are discriminated `oneOf`s,
  each branch requiring a member no other has.
- **Who implements it** (§2.1), **the served schema** (§2.2), **the
  read-back** with each value's source and startup value, the revision and
  the instance, put before the tokens (§2.3), and **the three classes**
  (§2.4).
- **One order of refusals** for `set` (§2.5), from the core's own to the
  device, with core §5.2's envelope and `ConfigError{restart-required,
  device-refused}`; and for the control operations (§2.10) and `persist`
  (§2.13).
- **Changes and tokens** (§2.6), **one pending change, joining, `busy`,
  and one act at a time** (§2.7), **the window and the floor** with time on
  the wire (§2.8), **hot and reach** with `apply_at` before the deadline
  (§2.9), **confirm, cancel and extend** (§2.10), **the undo, once** (§2.11),
  **a restart is a rollback and a new instance** (§2.12), **persist**
  (§2.13), **idempotency keys for 300 s** (§2.14), **sensitive values**
  (§2.15), **the startup allowlist** (§2.16), **the change event** and its
  rate (§2.17), **desired state** (§2.18), **the compact status** and
  constrained faces (§2.19), **device-as-service** (§2.20), and **access
  control** (§2.21).
- **What a tool may conclude** (§5), the changes against RFC 05 §5.1
  (Appendix A), and the adopters: zenoh-modem, the reference, and ROS 2
  parameters (Appendix B).
- **Evidence:** `conformance/validation.json` (99 cases),
  `conformance/window.json` (18), `conformance/lifecycle.json` (24 cases,
  113 steps), `conformance/desired.json` (28), `conformance/redaction.json`
  (9), and fourteen scenario sections, twelve the runtime's and two a
  deployment's and a tool's. `zenkey_model::config` runs every fixture.

**Decided on #768, before the text** (the maintainer's, not reopened
here): zenoh-modem's semantics whole; the resources above, each act its
own `@op`; `SetReply` as `{applied}` or `{pending}`; no operation
idempotent, `idempotency_key` kept 300 s, every operation
`fanout = "forbidden"`; the window as `max(confirm_s, floor)`, with the
floor and a per-group `writable` in the view; the errors mapped by the
binding's §5; a restart as a new instance; desired state refused at the
server; attribution as O7's, claimed only, with an optional
`claimed_source`; and three defaults: control operations answer the view,
or the status through `ControlReply` when asked; the change resource is an
event, `burst(600/h)`, `7d`; `apply_at` is the device's to state.

**Decided here, where the record was silent.**
- **`busy` names the token in its message, not a detail.** The binding's
  §5 and the decisions on #768 put the token "in the detail", and core
  §5.2 carries a detail on `app` alone: a tool refuses a detail on any
  other code. A program reads the token from `pending` in the view or the
  status, which it can GET; a person reads it in the message (§2.5, §2.7).
- **`persist` of the pending change is `busy`.** RFC 05 v1.50 let
  `persist` take the pending token, and its own rule makes a restart roll
  a pending change back: a persisted, unconfirmed change would come back
  after a restart. `persist` takes the last change alone (§2.13).
- **`persist` of a change that set a sensitive value is `forbidden`.**
  zenoh-modem refuses it (the binding's §4); the core's `forbidden` is the
  code for an act refused by rule rather than by the request's form or the
  device.
- **`persist` is optional**, so an owner without a persisted layer says so
  in its descriptor, with cause `config` (core §3.3, O3), and a tool sees
  it before it calls.
- **The order of refusals** (§2.5): an act in flight, then an idempotency
  key, before the schema; the schema before the allowlist; the pending
  change before the request's guards; the guards before the values, taken
  in bytewise order; the rate and a reach join's deadline before the
  device. A retry comes before the `busy` its own first attempt would
  cause.
- **One act at a time,** and `busy` meanwhile (§2.7): a confirm racing the
  deadline's undo is told, never applied half.
- **The deadline belongs to the undo** (§2.11): a confirm received at the
  deadline is too late, `busy` until the undo has run, `not_found` after.
- **The revision counts acts** that moved running values (§2.3), and
  restarts at 0 with each instance; the view and the status carry the
  instance, since the revision crosses faces where presence does not
  (core §1.5).
- **`startup` is what a restart would give** (§2.3): the persisted
  layer's, else the file's, else the default. RFC 05's "its startup value
  where the two differ" did not say which layer.
- **A reach apply that fails** after its reply undoes the change at once
  (§2.9), and **an undo with nothing applied** is no act (§2.11).
- **Joining**: a hot group applies at once, a reach group at its own
  `apply_at`, which must precede the deadline, else `busy` until the
  caller extends (§2.7, §2.9). A group may join twice; the undo restores
  each parameter to its value before the change.
- **Edits carry their group** (§2.17): a joined change spans groups, whose
  parameter names may repeat. An event is attributed to the `set` that
  started its change.
- **The event's rate** is kept by refusing `busy` a `set` that would start
  a change while 600 events fall within the last hour; with one pending
  change at a time, no change's end event can break it (§2.17).
- **An idempotency key keeps refusals too**, from §2.5's row 3 on, and a
  key reused for another request is `invalid_request` (§2.14). A retry
  that meets its own first attempt's act in flight is `busy`, and the
  attempt's answer is kept when the act ends: a refusal sent before the
  owner reads the key is not kept.
- **An integer is written without a fraction or an exponent** (§2.5), as
  core §3.3 reads one in a descriptor: stricter than JSON Schema 2020-12,
  for which `2.0` is an integer, and an owner's further check that core
  §5.1 allows.
- **Desired state**: a `DesiredConfig` shape for authors, refused whole
  with the first problem in a stated order, converged by `set`, and
  reported as `desired` in the view and the status (§2.18).
- **Time on the wire** is RFC 3339 in UTC, the owner writing milliseconds
  (§2.8), as v1's `apply_at` was a string.
- **`service` replaces v1's `resource`** in the view and the event: in zk2
  a resource is a contract's template, and the configured resource is the
  service.

**Deliberately not stated.**
- **No core amendment.** Core §7.3's `oneOf` already expresses the
  discriminated unions of architecture D11: no discriminator keyword is
  needed, since each branch requires a member no other branch has. Core
  §8.2 already lets a profile make put-before-tokens a MUST, and core §2.7
  already binds the owner to the event's rate.
- **No upper bound on a window,** and no maximum number of idempotency
  keys: the rate bounds the keys to 50 within 300 s.
- **No `answer = "status"` for `set`.** `SetReply`'s branches were
  decided; a reach `set`'s reply is already small, and a third branch is a
  later minor (core §9.8 classifies a `oneOf` branch added as a review).
- **No idempotency key for control operations**: the caller reads the last
  change instead (§2.10, §5).
- **No guard of `expected_revision` across instances:** the revision
  restarts with each instance, and a caller that read another instance's
  view sees it in the `instance` it read.
- **No float, byte or array kind** (§2.2; Appendix B's ROS 2 doubles).
- **No runtime and no tool here.** The runtime and the owner example are
  #768's CB, zenctl's `config` verbs and `check conform` its CC, zengui's
  Config tool its CD. They run `scenarios.md` §1–§12 and §13–§14.
