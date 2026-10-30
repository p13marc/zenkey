# `desired.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §8, §10 and §11 are the
runtime's: the reference runs them as tests of `zenkey`
(`zenkey/tests/profile_desired.rs`, one test per section, named after
it). §9 is a tool's: the reference tool runs it as a test of zenctl
(`zenctl/tests/live_zk2.rs`, `desired_s9_…`, T being `zenctl desired`,
the doctor's `desired-*` checks beside it).

**Conventions.**
- **Section numbers** in parentheses (§2.7) are v1.md's rules; core
  sections say "core".
- **The bus.** R1 is a router with timestamping on, as a router's default
  is. Where a section cuts a link, the author's side is R1 and the
  target's side R2, a second router linked to R1 through a path the test
  can cut and heal (spike S12 cut a proxy). Everything else is a client
  of the router on its side. Timeouts are 1 s unless stated.
- **The contracts.** Every section uses `tasking.v1`, the author's, and
  `worker.v1`, the target's. The reference keeps them as
  `zenkey/tests/contracts/tasking.v1.toml` and `worker.v1.toml`, beside
  `tasking.json`, and lists the profile's file by its path from there:

  ```toml
  # tasking.v1.toml: a task per worker, state of its author
  [interface]
  name  = "tasking"
  major = 1
  minor = 0
  uses  = ["desired.v1", "freshness.v1"]

  [schemas]
  jsonschema = ["tasking.json"]

  [resources."tasks/{system}/{service}"]
  kind        = "state"
  type        = "json:Task"
  params      = { system = "string", service = "string" }
  cardinality = 16
  annotations = { "desired.target_param" = ["system", "service"], "freshness.ttl_s" = 0 }
  ```

  ```toml
  # worker.v1.toml: what a worker runs, its report, and its role
  [interface]
  name  = "worker"
  major = 1
  minor = 0
  uses  = ["desired.v1"]

  [schemas]
  jsonschema = ["../../../spec/profiles/desired/desired.v1.json", "tasking.json"]

  [resources.running]
  kind = "state"
  type = "json:Task"

  [resources."desired/task"]
  kind = "state"
  type = "json:desired.v1.Report"

  [requires.task]
  interface = "tasking.v1"
  resources = ["tasks/{system}/{service}"]
  optional  = true
  ```

  `tasking.json` is, byte for byte, the block below without the
  list's two-space indent, its last line ending in a line feed:

  ```json
  {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "tasking.json",
    "$comment": "desired.v1's scenarios (spec/profiles/desired/scenarios.md): the author's document type. An open object: a worker refuses a document that holds a member named endpoint (reach), and one whose rate_hz is above what it supports (invalid).",
    "$defs": {
      "Task": {
        "type": "object",
        "properties": {
          "revision": { "type": "integer", "minimum": 0 },
          "mode": { "type": "string", "enum": ["idle", "run", "halt"] },
          "rate_hz": { "type": "integer", "minimum": 1, "maximum": 1000 }
        },
        "required": ["revision", "mode", "rate_hz"]
      }
    }
  }
  ```

  It defines one type, `Task`, an open object: `revision`, an integer
  from 0, required (the type's revision, §2.4); `mode`, one of `idle`,
  `run`, `halt`, required; `rate_hz`, an integer from 1 to 1000,
  required. Its artifact's id is the hash of its JCS bytes (core §7.1):
  whitespace and member order aside, a file that differs anywhere, its
  `$comment` included, holds another `Task`, and gives `tasking.v1` and
  `worker.v1` other fingerprints. The worker supports `rate_hz` up to
  100 and refuses a higher one `invalid` at `/rate_hz`; it refuses a
  document that holds a member named `endpoint` `reach` at `/endpoint`
  (§2.7): the type gives no such member, and the object is open.
- **The author** is `lab/author`, implementing `tasking.v1`, its tombstone
  window 60 s unless stated (core S3), with a persistent record of the
  documents it put, each with its stamp (the reference: a file).
- **The target** is `lab/worker`, implementing `worker.v1`, its role
  `task` bound to `lab/author` with `{system} = self.system` and
  `{service} = self.service` (§2.3). Its slice is
  `D = zk2/lab/author/tasking.v1/state/tasks/lab/worker`, its report
  `Rp = zk2/lab/worker/worker.v1/state/desired/task`, and what it runs
  `Run = zk2/lab/worker/worker.v1/state/running`. Its baseline is
  `{"revision": 0, "mode": "idle", "rate_hz": 1}`. Its drive is
  simulated: a test can make an act fail after it set the rate and
  before it set the mode, and can hold an act from its start until the
  test releases it.
- **Documents.** `v<n>` is a `Task` with `revision` *n*, `mode` `run` and
  `rate_hz` 10 unless a step says otherwise, put by the author on `D`;
  `s<n>` is the stamp its put carries.
- **S** subscribes to `zk2/lab/**` before anything starts, and to the
  tokens under `zk2/lab/*/@zk/**` by liveliness (core §1.3: `**` never
  crosses `@zk`), and records each delivery's arrival on its monotonic
  clock, its key, its kind, its payload and its stamp. Where a section
  cuts a link, S is a client of R1, and **S2**, a client of R2,
  subscribes to `Rp` and `Run` before anything starts and records as S
  does. S2 sees the target's side while the link is cut, which S cannot,
  and zenoh replays nothing at the heal.
  **G** GETs as core S4 says. "The report" is the
  latest of `Rp` by its stamp; "converged on v*n*" means the report has
  `source` `desired`, `in_force` and `observed` both `{stamp: s<n>,
  revision: n}`, `outcome` `converged`, and `Run` holds v*n*'s content.
- **The owner example.** The reference's author and target are
  `zenkey/examples/owner.rs` run with `--desired-author` and
  `--desired-target lab/author` (`--desired-unbound` for §8), and
  `owner --help` is the interface a black-box runner drives, as for
  `alarms.v1`: its drive is the one above, `desired hold`, `release` and
  `fail-next` its knobs. A target says what its last seed found
  (`desired seeds`: the replies its GET drew, the author's presence or a
  read not made, the archive), and an author how many GETs reached a
  target's slice (`desired gets`), which §8 and §10 observe.

## §1 Converge after an outage (§2.5, §2.6, §2.10, §2.15)

**Setup.** The author on R1, the target on R2, the link up. The author
holds `tasking.v1` in its tokenless set (core §8.1): it holds no
`alive/tasking.v1/…` token, and its descriptor marks the interface
`"token": false`.

**Steps.**
1. The author starts with no document. The target starts. G GETs `Rp`
   the moment S sees the target's instance token.
2. The author puts v1.
3. The test cuts the link between R2 and R1. The author puts v2. S, on
   R1, waits 2 s.
4. The test heals the link.

**Expected.**
1. G's reply holds `{key: D, source: baseline}`: the target put it
   before its tokens (§2.10). `Run` holds the baseline.
2. S receives a put on `D` stamped by the author (core S1), then on `Rp`
   a report converged on v1.
3. Read through S2: `Rp` and `Run` receive nothing during the cut.
   The target's subscription received nothing of v2, so it acted on
   nothing, and its report is unchanged.
4. The target sees the author's instance token appear again, seeds (core
   §4.2), and acts on v2 (§2.5, §2.6): its report is converged on v2
   within 5 s of the heal (spike S12: 1.4 to 1.6 s). It acted on v2 once.
   At no step did the target wait on an interface token of the author's,
   which never appeared (§2.15).

## §2 Never an older version (§2.6, §2.7)

**Setup.** The target converged on v2, `s2` its stamp. A test session X,
a client of R1, can put on `D` with a timestamp it sets. X takes `s2` as S
received it, the timestamp of the author's put of v2, and builds each
timestamp below from that one: a binding may not build every
NTP64 time from its text, and `s2`'s fraction must survive exactly.

**Steps.**
1. X puts v1's content on `D` with the timestamp `s2`'s time less 1 s,
   under `s2`'s id.
2. X puts v3's content on `D` with the timestamp S received with `s2`,
   exactly.
3. The target re-reads on its deployment's period, which the test sets
   to 2 s for this step (§2.5).

**Expected.**
1. The target acts on nothing: an earlier time of the held id is older
   (§2.6). `Rp` and `Run` receive nothing. A GET of `Rp` holds the report
   converged on v2.
2. Nothing: an equal stamp is the same version, never acted on again.
3. The author answers the re-read with v2 under `s2` (core S2): the same
   version, nothing acted on, nothing put on `Rp`.

## §3 A delete reverts, within the window (§2.8)

**Setup.** The author's tombstone window is 10 s for this section. The
author on R1 and the target on R2, the link up. The target converged on
v1.

**Steps.**
1. The author deletes `D`.
2. The author puts v2.
3. The test cuts the link. The author deletes `D`. 3 s later, the test
   heals the link.
4. The author puts v3. The test cuts the link, the author deletes `D`,
   and the test heals the link 15 s later, past the window.

**Expected.**
1. The target reverts: `Run` holds the baseline, and the report is
   `{key: D, source: baseline, observed: {stamp: <the delete's>},
   outcome: reverted}` (§2.8, §2.10).
2. The report is converged on v2.
3. The target's seed after the heal holds a `reply_del` for `D` with the
   delete's stamp (core S3), and the target reverts, as in 1.
4. The target's seed after the heal draws nothing for `D`: past the
   window, the author no longer answers the delete. The target keeps v3
   in force, and its report stays converged on v3: silence is no verdict
   (§2.8, core O5). This is the outage §2.8's window must cover, and a
   deployment that configures a shorter one gets this.

## §4 A refused document keeps the previous in force (§2.7, §2.10)

**Setup.** The target converged on v1.

**Steps.**
1. The author puts v2 with a member `"endpoint": "tcp/10.0.0.9:7447"`.
2. The author puts v3 with `rate_hz` 500.
3. The author's instance is re-minted in its own session (core §8.1),
   keeping its zid, so it puts v3's content again from its new instance,
   under its zid and a later time (§2.11), and the target seeds.
4. The author puts v4.
5. The test sets the target's drive to fail its next act. The author puts
   v5 with `rate_hz` 20 and `mode` `halt`.

**Expected.**
1. The target applies nothing: `Run` still holds v1, and the report is
   `{key: D, source: desired, in_force: {stamp: s1, revision: 1},
   observed: {stamp: s2, revision: 2}, outcome: refused, refusal: {code:
   reach, pointer: /endpoint, …}}`.
2. The same, with `observed` v3 and `refusal` `{code: invalid, pointer:
   /rate_hz, …}`.
3. The re-put carries v3's document under a later stamp of the same id,
   `s3`'s: adopted, never refused again (§2.6 step 3, §2.7). The target
   applies nothing, and its report moves `observed` to the re-put's
   stamp, `revision` 3, still `refused` with the same refusal, `in_force`
   still v1. The seed's answer is that re-put: the same version, nothing
   more.
4. The report is converged on v4.
5. `Run` holds `rate_hz` 20 and `mode` `run`, and the report is
   `{key: D, source: desired, observed: {stamp: s5, revision: 5},
   outcome: failed, refusal: {code: apply, …}}`, with no `in_force`
   (§2.7, §2.10).

## §5 A restart acts again (§2.7, §2.5)

**Setup.** The target converged on v1. The test holds the target's next
act from its start: the author holds v1 throughout, so the new
run's first seed draws it at once, and an act left to run could put
a converged report before G's GET reaches the target.

**Steps.**
1. The target's process ends without closing. It restarts, a new run
   (§0) with a new instance. G GETs `Rp` the moment S sees its new
   instance token. When G has its reply, the test releases the held act.

**Expected.**
1. G's reply holds `{key: D, source: baseline}` under the new instance's
   stamp (§2.10): the new run holds nothing of the previous run's acts.
   The target seeds, acts on v1 again, and its report is converged on v1,
   under `s1`, the version its previous run acted on (§2.7).
   S receives a put on `Run` holding v1's content from the new instance.

## §6 The author's restart moves no target to act again (§2.7, §2.11)

**Setup.** The target converged on v1. A second slice of the author's,
`D2 = zk2/lab/author/tasking.v1/state/tasks/lab/spare`, held a document
the author deleted 5 s before step 1, inside its 60 s window, and its
record holds that delete.

**Steps.**
1. The author's process ends without closing. It restarts from its
   record, a new instance in a new session with a fresh zid (core §1.5).
   G GETs `D` and `D2` the moment S sees the author's new instance token.
   S listens 3 s.
2. The author changes its document: it puts v2.
3. The author's process ends without closing, and its record is removed.
   It restarts with no record: a new epoch, a fresh zid (core §4.3). Its
   source yields v2's content again, which it puts.
4. The author changes its document again: it puts v3.

**Expected.**
1. Before its new instance token (a SHOULD; before G's GET, a MUST), S
   receives a put on `D` holding v1's content, unchanged, under a stamp
   whose id is the author's new zid, and a delete of `D2` under the same
   id (§2.11). G's GET holds that put for `D` and a `reply_del` for `D2`,
   each stamped by the new instance, whose `meta.zid` is their id. The
   target adopts the re-put without an act: `Run` receives nothing, and S
   receives on `Rp` a report with `observed` and `in_force` the re-put's
   stamp, `revision` 1, still `converged` (§2.7). The target acted on
   nothing.
2. `s2` is later than the re-put's stamp, and the report is converged on
   v2, which the target acted on.
3. S receives a put on `D` holding v2's content under a stamp of the new
   zid: another id (§2.6), the same document, adopted. `Run` receives
   nothing, and the report moves to that stamp, still `converged`, still
   `revision` 2. A GET of `D2` draws nothing: its delete was lost with the
   record (§2.11).
4. The report is converged on v3, which the target acted on.

## §7 Last-known from an archive, the author absent (§2.9)

**Setup.** An archive, `lab/archive` (core §4.4), on the target's side,
records `zk2/lab/author/tasking.v1/state/**`. The author puts v1, the
target converged on it, and the archive recorded it.

**Steps.**
1. The author closes cleanly. The target's process ends, and restarts.
2. The author restarts from its record.

**Expected.**
1. The target seeds: a complete presence read shows no instance token of
   the author's, which is absent (core §8.1), and its GET draws no reply.
   The target reads `D`'s archive form,
   `zk2/lab/archive/archive.v1/@state/lab/author/tasking.v1/state/tasks/lab/worker`
   (core §4.4, S5), acts on v1 under `s1`, and its report is converged on
   v1 with `last_known: true` (§2.9).
2. The restarted author puts v1's document again from its new instance
   (§2.11), under a stamp of its new zid: another id than `s1`'s, the
   same document. The target receives it by its subscription, or as the
   answer to the seed it makes when it sees the author's instance token,
   and **adopts** it (§2.6, §2.7). It acts on nothing (`Run` receives
   nothing), and its report moves `observed` and `in_force` to that stamp,
   `revision` 1, still `converged`, without `last_known` (§2.10). A
   delivery of the same re-put by the other path is the same version.
   Confirmation, the same stamp from its author, is §10's.

## §8 The kill switch (§2.12)

**Setup.** The target's role `task` is configured unbound. The author
holds v1 on `D`.

**Steps.**
1. The target starts. G GETs its descriptor and `Rp` the moment S sees
   its instance token. The test waits 3 s.

**Expected.**
1. The descriptor lists `task` with `"bindings": []` and `"params": {}`
   (core §3.2). `Rp` holds `{source: baseline}`, with no key (§2.10,
   §2.12). `Run` holds the baseline, and stays so: the target makes no
   GET of `D` (the author's queryable receives none: the owner example's
   `desired gets lab/worker` counts them) and acts on nothing.

## §9 A tool's questions (§2.17, §5)

**Setup.** §1's bus. T reads the target's descriptor and contracts, finds
its reports by type identity (§2.10), takes the one whose `key` is `D`,
GETs `D` from the author (core S4), and asks §5's four questions. With its
author absent, T has no descriptor of the author's to read its contract
by, and takes `D` from the target's requirement and binding (§2.17).
The reference's T is `zenctl desired`, a judgement: exit 0 every role
asked converged on its author's current document, 1 a refused or failed
act or a target behind past its window, 2 no verdict.

**Steps.**
1. After §1's step 4, T asks.
2. After §4's step 1, T asks.
3. After §4's step 5, T asks.
4. After §7's step 1, T asks.
5. Against §8's target, T asks.
6. After step 3, the author puts v5, and the target converges on
   it. The test holds the target's next act from its start, and the
   author puts v6. T asks once; then T asks again over a window of 2 s,
   reading as often as it likes.

**Expected.**
1. Converged: **yes**, `converged`. Refused: **no**, `converged`.
   Last-known: **no**, `current`. In force: **yes**, `desired`.
2. Converged: **no**, `refused`. Refused: **yes**, `refused`.
   Last-known: **no**, `current`. In force: **yes**, `desired`.
3. Converged: **no**, `failed`. Refused: **yes**, `failed`. Last-known:
   **no**, `current`. In force: **yes**, `partial`.
4. Converged: **not asked**, `author_absent`. Refused: **no**,
   `converged`. Last-known: **yes**, `last_known`. In force: **yes**,
   `desired`. T shows the archive's v1 as last-known, never current.
5. Converged, refused and last-known: **not asked**, `unbound`. In force:
   **no**, `unbound`.
6. Once: converged **not yet**, `behind`, the report's `observed` v5's
   stamp and the author's v6's; refused **no**, `converged`. No verdict: a
   reading alone never finds a target behind. Over the window, every
   reading is not yet on v6: T finds the target behind v6 for at least
   2 s (§2.17), its finding, §5's answer still **not yet**. With a
   clock trusted to the HLC delta, T may find it at the first reading of
   a window once v6 is older than the window.

## §10 Last-known confirmed by its author; a present author's silence (§2.5, §2.9)

**Setup.** An archive, `lab/archive` (core §4.4), on the target's side,
a client of R2, records `zk2/lab/author/tasking.v1/state/**`. The author
on R1 and the target on R2, the link up. The author puts v1, the target
converged on it, and the archive recorded it.

**Steps.**
1. The test cuts the link between R2 and R1. The target's process ends
   without closing, and restarts, a new run.
2. The test heals the link.
3. The test cuts the link again. The author puts v2, which
   neither the target nor the archive receives. X, a client of R2,
   declares an instance token at the author's address,
   `zk2/lab/author/@zk/instance/<an id of its own>`, and no queryable: the
   author present on R2, its queryable silent there. The target's process
   ends without closing, and restarts, a new run.
4. The test heals the link. Once the author's own instance token is on R2,
   X withdraws its token.

**Expected.**
1. The target seeds: no instance token of the author's crosses the cut,
   so a complete presence read through R2 shows none, absence by core
   §8.1, and its GET draws no reply within the
   timeout. The target reads `D`'s archive form,
   `zk2/lab/archive/archive.v1/@state/lab/author/tasking.v1/state/tasks/lab/worker`
   (core §4.4, S5), acts on v1 under `s1`, and, read through S2, its
   report is converged on v1 with `last_known: true` (§2.9).
2. The author's instance runs throughout, so its instance token appears
   again on R2 (core §8.1). The target seeds, and the author answers v1
   under `s1`, its own stamp: the same version, **confirmed** (§2.6 step
   3, §2.9). The target acts on nothing (`Run` receives nothing), and puts
   its report again without `last_known`, `observed` and `in_force` still
   `{stamp: s1, revision: 1}` (§2.10).
3. The target seeds: a complete presence read through R2 shows an
   instance token at the author's address, X's, so the author is present,
   and its GET draws nothing within the timeout: silent, no verdict
   (§2.5). The archive holds v1, older than the author's v2. The target
   reads no archive while its author is present, as the reference does
   (§2.5), and says so (`desired seeds`: no reply, the author present, no
   archive read); it acts on nothing, and, read through S2, its report
   stays `{key: D, source: baseline}` and `Run` the baseline. A target
   that takes §2.5's MAY on a silent GET acts on the archive's v1 instead,
   `last_known: true`, a version its author has replaced: what the
   reference's choice avoids.
4. The author's instance token appears on R2; the target seeds, the
   author answers v2 under `s2`, and the target acts on it: its report is
   converged on v2, without `last_known`. X's withdrawal moves nothing.

## §11 A role on two documents refuses the start (§2.1, §2.3)

**Setup.** One router, R1, and no author: the refusal reads the author's
declaration, which the target holds, the contract it is compiled against
(core R4). Two copies of the contracts:
- **tasking.v1, two documents**: `tasking.v1.toml` with a second desired
  resource, declared as the first is:

  ```toml
  [resources."plans/{system}/{service}"]
  kind        = "state"
  type        = "json:Task"
  params      = { system = "string", service = "string" }
  cardinality = 16
  annotations = { "desired.target_param" = ["system", "service"], "freshness.ttl_s" = 0 }
  ```

- **worker.v1, both**: `worker.v1.toml` whose `[requires.task]` names
  both, `resources = ["tasks/{system}/{service}",
  "plans/{system}/{service}"]`.

**Steps.**
1. The target, `lab/worker`, implements worker.v1, both, its role `task`
   bound as "The target" says, against tasking.v1, two documents, and
   starts. The test waits 5 s.
2. The control: the target implements the scenarios' worker.v1, its
   requirement naming `tasks/{system}/{service}` alone, against
   tasking.v1, two documents, and starts.

**Expected.**
1. The role is `two_documents` (§2.3), and the target refuses to start,
   as core §8.2 step 2 refuses an invalid declaration: nothing of it is
   declared. S sees no instance token of `lab/worker` within the 5 s, and
   no report on `Rp`. The reference's start returns the refusal, `not
   started: role "task": its binding is invalid (two_documents)
   (desired.v1 §2.3)`, and the owner example given the two copies stops
   with it, exit 1. A target that starts without the role fails this
   step: S sees its instance token.
2. S sees the target's instance token: a requirement naming one of the
   author's two desired resources is a role on one document type, and
   the target starts (§2.1). Its report on `Rp` is
   `{key: zk2/lab/author/tasking.v1/state/tasks/lab/worker, source:
   baseline}` (§2.10).
