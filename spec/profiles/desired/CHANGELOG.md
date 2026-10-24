# `desired.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.1 — 2026-10-24: the first text, draft (#840, DA)

`desired.v1` is the profile that three places defined each on its own:
`config.v1` §2.18's `DesiredConfig`, whose CHANGELOG defers to it three
times (its 0.1's and 0.3's entries: drift, and its own words); the
walkthrough's `mission_plan.v1`, with r4 §4.3's executor and spike S12's
measurement; and ZenSight's `zs.desired.v1`, whose v1 implementation
(`zensight-sensor-core/src/desired.rs`, `zensight-desired`) is the target.
Core Appendix D's interim row (`desired | target_param, target`) was all
the core said. This text is written against core 0.37, which publishes the
vocabulary and lets an author's restart keep its zid.

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
  **reading** (§2.5), **the order of versions** (§2.6), **acting**
  (§2.7), **absence, deletes and cancellation** (§2.8), **last-known**
  (§2.9), **the report** with its consistency rules (§2.10), **the
  author** (§2.11), **the kill switch** (§2.12), **population** (§2.13),
  **constrained faces** (§2.14), **the tokenless set and access control**
  (§2.15), **`config.v1` as a target** (§2.16), and **what a tool
  concludes of one target** (§2.17).
- **The standard schema,** `desired.v1.json` (§3), artifact
  `sha256:0b249e71…`: `desired.v1.Report`, `desired.v1.Version` and
  `desired.v1.Refusal`, every object open.
- **What a tool may conclude** (§5): has the target converged on its
  author's current document; is its last act refused or failed; is what
  it acted on last-known; is a desired document in force. The changes
  against v1's `@desired` plane (Appendix A), and the adopters' mappings
  (Appendix B).
- **Evidence:** `conformance/declarations.json` (36 cases),
  `conformance/bindings.json` (20), `conformance/order.json` (45),
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
- **A horizon above 0 makes a declaration invalid.** `freshness.v1` §2.4
  would have the author re-put every document within half of it, and a
  re-put is a new stamp, which every target acts on again. D16's
  `ttl_s = 0` is a SHOULD, not a MUST: the walkthrough's contract declares
  no horizon, which `freshness.v1` §2.3 reads as "not asked" and which
  asks no re-put either.
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
- **When an act is recorded.** A version is acted on once from the
  moment its act starts, so a re-delivery while it runs is the same
  version. While an act runs, the target holds the latest admitted
  version, the latest replacing an earlier one held, and acts on it next:
  the target converges on the latest, and a version passed over while it
  waited was never acted on, which is not "older".
- **A rejected version is not tried again.** ZenSight's v1 sensor left a
  rejected sample's timestamp unapplied, so every re-GET re-rejected it;
  here the act is recorded, whatever its outcome, as `config.v1` §2.18
  does. The next version is the author's to make.
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
  `secret` and `invalid`, and `apply` for a failure; a type MAY use its
  own, as `config.v1`'s are, and a reader shows a code it does not know.
  `secret` is added beside reach: a document is published to every reader
  granted it and kept by archives, which `config.v1`'s `sensitive` and
  ZenSight's never-list both refuse. A `pointer`, a JSON Pointer, says
  where, which `config.v1`'s group and parameter map onto.
- **A delete reverts only on positive evidence.** A target reverts on a
  delete or a `reply_del`, never on a GET that drew nothing for its key:
  that is how core §4.4's archive drops a key, and an empty reply set is
  no verdict (O5). A delete lost past the window therefore leaves the
  previous document in force (`scenarios.md` §3), which is what the
  window's MUST prevents.
- **Last-known is confirmed by the author.** When the author's answer
  later carries the version the target acted on from an archive, the
  target holds it as the author's and reports so, without acting again.
- **The author's restart answers with its recorded stamps.** The issue's
  "writes nothing unchanged" needs the restarted author to answer its
  targets' seeds with the versions they acted on: each document under the
  stamp its record holds, which core S2 allows (a reply carries the stamp
  of the mutation it represents). A tool's S1 check (core §4.2) reads a
  stamp as the owner's by `meta.zid`, so the author SHOULD keep its zid
  across such a restart; core 0.37 says that core §1.5's new session is
  for an owner whose state's ordering restarts. `alarms.v1` §2.9 re-puts
  instead, which a desired document cannot: a re-put is a version.
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
