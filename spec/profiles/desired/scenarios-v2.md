# `desired.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §12 are
[`desired.v1`'s scenarios](scenarios.md), restated where the version or
the read changes. §13 to §15 are new. §9 is a tool's, and the others
the runtime's.

**No implementation runs them yet.** The reference is to run them as
tests of `zenkey` (`zenkey/tests/profile_desired_v2.rs`, one test per
section, named after it), and §9 as a test of `zenctl`.

**Conventions.**
- **Section numbers** in parentheses (§2.7) are v2.md's rules; `v1 §n`
  is a section of `desired.v1`'s text, and core sections say "core".
- **By reference.** A section that says "as v1's" is
  [`scenarios.md`](scenarios.md)'s section of the same number, with the
  contracts, the keys and the readings below. Where v1's names a stamp,
  it reads a version. Where it names a `reply_del`, it reads a
  tombstone.
- **The bus** is v1's: R1, and R2 where a section cuts a link. Timeouts
  are 1 s unless stated.
- **The contracts.** Every section uses `tasking.v2`, the author's, and
  `worker.v2`, the target's. The reference is to keep them as
  `zenkey/tests/contracts/tasking.v2.toml` and `worker.v2.toml`, beside
  v1's `tasking.json`, unchanged:

  ```toml
  # tasking.v2.toml: a task per worker, state of its author
  [interface]
  name  = "tasking"
  major = 2
  minor = 0
  uses  = ["desired.v2", "freshness.v2"]

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
  # worker.v2.toml: what a worker runs, its report, and its role
  [interface]
  name  = "worker"
  major = 2
  minor = 0
  uses  = ["desired.v2"]

  [schemas]
  packed     = ["../../../spec/profiles/desired/desired.v2.json"]
  jsonschema = ["tasking.json"]

  [resources.running]
  kind = "state"
  type = "json:Task"

  [resources."desired/task"]
  kind = "state"
  type = "packed:Report"

  [resources."desired/task".annotations]
  "desired.report"   = "task"
  "desired.pointers" = ["", "/mode", "/rate_hz", "/endpoint", "/revision"]
  "desired.details"  = ["above what the drive supports", "names how the worker reaches the bus", "a revision above 4,294,967,295"]

  [requires.task]
  interface = "tasking.v2"
  resources = ["tasks/{system}/{service}"]
  optional  = true
  ```

  The worker refuses as v1's does: `rate_hz` above 100 is `invalid` at
  `/rate_hz`, index 2, with line 0. A member named `endpoint` is `reach`
  at `/endpoint`, index 3, with line 1.
- **tasking.v2, watched** is `tasking.v2.toml` with `"freshness.ttl_s" =
  4`. §14 uses it. Its author sends a heartbeat covering `tasking.v2` at
  most every 2 s (`freshness.v2` §2.4), on
  `Hb = zk2/lab/author/freshness.v2/stream/heartbeat/tasking.v2`.
- **The author** is `lab/author`, implementing `tasking.v2`, its
  tombstone window 60 s unless stated. It keeps a record of the
  documents it put, and of its deletes within the window. It keeps its
  run's record too, so it writes under 16-bit runs (core §4.3).
- **The target** is `lab/worker`, implementing `worker.v2`, its role
  `task` bound as v1's is. Its slice is
  `D = zk2/lab/author/tasking.v2/state/tasks/lab/worker`, its report
  `Rp = zk2/lab/worker/worker.v2/state/desired/task`, and what it runs
  `Run = zk2/lab/worker/worker.v2/state/running`. Its baseline and its
  drive are v1's.
- **Documents.** `v<n>` is v1's. `s<n>` is the version its put carries:
  the run and the seq in its envelope (core §7.5).
- **S**, **S2** and **G** are v1's. S records each delivery's in-band
  stamp. A GET of `D`, `Rp` or `Run` draws a snapshot (core §4.2): one
  page, whose one entry has the empty id.
  - "The report" is `Rp`'s record, read as §2.10 says, the latest by
    seq within its run.
  - "The baseline's report" is `source` `baseline`, `not_acted`, and
    nothing more: the 3 bytes `000000`.
  - "Converged on v*n*" means the report has `source` `desired`,
    `outcome` `converged`, `in_force` `observed`, the observed version
    `s<n>` and `revision` *n*, and `Run` holds v*n*'s content.
- **What a section observes of a read.** The author says, for each GET
  of a slice, the held stamp it carried, or none, and whether it
  answered with that stamp alone (the owner example's `desired gets`).
  The target says what its watch expects (`desired watch`).
- **A played author.** In §13 a session of the test's own, P, plays the
  author, as core `state.md` §16's plays an owner. It holds an instance
  token at `lab/author` and a queryable on `D`. It puts nothing, and
  answers each GET as the step says, under a 16-bit run R.

## §1 Converge after an outage (§2.4, §2.5, §2.6, §2.10, §2.15)

As v1's, with these readings.

**Expected.**
1. G's reply is a snapshot of `Rp` whose entry is the baseline's report,
   under tag 12.
2. S receives a put on `D` in a mutation's envelope (core §7.5). Where
   it carries a Zenoh timestamp, the id is R1's, never the author's
   (core §4.2, "Observing S1"). G's GET of `D` draws a snapshot: one
   live entry, whose stamp is that put's run and seq. Then `Rp` holds a
   report converged on v1.
3. As v1's.
4. The target's seed after the heal carries the held stamp `s1`. The
   author's last mutation of `D` is v2's, so it answers whole. The
   target acts on v2 once, and its report is converged on v2 within 5 s
   of the heal.

## §2 Never an older version (§2.6, §2.7)

**Setup.** The target converged on v2, `s2` its version, of the author's
run R at seq n. A test session X, a client of R1, can put on `D` with an
envelope it writes.

**Steps.**
1. X puts v1's content on `D` under R at seq n − 1.
2. X puts v3's content on `D` under R at seq n, `s2` exactly.
3. The target re-reads on its deployment's period, which the test sets
   to 2 s for this step.
4. X puts v3's content on `D` under a 32-bit run of its own, at seq 0.
5. X puts v1's content on `D` under R at seq n − 1 again.

**Expected.**
1. The target acts on nothing: a lower seq of the held run is older.
   `Rp` and `Run` receive nothing.
2. Nothing: an equal seq is the same version.
3. The re-read carries the held stamp `s2`, and the author answers with
   that stamp alone. Nothing is acted on, and nothing is put on `Rp`.
4. Another run is admitted, and its document differs: the target acts on
   it. Its report is converged on v3, under X's version.
5. Nothing: R is a run the target remembers, and seq n − 1 is below the
   highest it acted on under R, whichever run it holds.

## §3 A delete reverts, within the window and past it (§2.8)

**Setup.** As v1's: the author's tombstone window is 10 s.

**Steps.** As v1's, 1 to 4.

**Expected.**
1. The target reverts. `Run` holds the baseline. The report is `source`
   `baseline`, `outcome` `reverted`, the observed version the delete's,
   with no revision.
2. The report is converged on v2.
3. The target's seed after the heal carries the held stamp `s2`. The
   author answers a snapshot whose one entry is a tombstone, with the
   delete's stamp. The target reverts, as in 1.
4. The target's seed after the heal carries the held stamp `s3`. The
   author holds no member at `D` any more, so it answers a snapshot with
   no entry, under the run it put v3 under. The target holds v3 of that
   run, at or below the snapshot's asof. **It reverts.** `Run` holds the
   baseline. The report is `source` `baseline`, `outcome` `reverted`,
   with no observed version. Under `desired.v1` this target kept v3 in
   force.

## §4 A refused document keeps the previous in force (§2.4, §2.7, §2.10, §2.11)

**Setup.** The target converged on v1.

**Steps.** As v1's, 1, 2, 4 and 5, and:

3. The author's instance is re-minted in its own session (core §8.1).
   It puts v3's content again from its new instance, and the target
   seeds.
6. The author puts v6 with `revision` 4294967296.

**Expected.**
1. The target applies nothing. `Run` still holds v1. The report is
   `source` `desired`, `outcome` `refused`, the observed version `s2`
   with `revision` 2, `in_force` `other` naming `s1` with revision 1,
   `code` `reach`, `pointer` 3 and `detail` 1.
2. The same, the observed version `s3`, `code` `invalid`, `pointer` 2
   and `detail` 0.
3. The author's new instance takes a new run. It puts v3's content
   again under it. That is another run's version of the document the
   target last acted on: adopted, never refused again. The report's
   observed version moves to the new one, still `refused`, with the same
   code, place and line. `in_force` still names `s1`.
4. The report is converged on v4.
5. `Run` holds `rate_hz` 20 and `mode` `run`. The report is `source`
   `desired`, `outcome` `failed`, the observed version `s5`, `code`
   `apply`, and `in_force` `none`.
6. The target refuses v6 whole, `invalid`, at `/revision`: `pointer` 4
   and `detail` 2. The report's observed version is `s6`, and it carries
   no `revision`. `Run` is unchanged.

## §5 A restart acts again (§2.7, §2.5)

As v1's. G's reply holds the baseline's report under the new instance's
run. The new process seeds with no held stamp, acts on v1 again, and its
report is converged on v1 under `s1`.

## §6 The author's restart moves no target to act again (§2.6, §2.11)

**Setup.** As v1's: the target converged on v1, and `D2`, a second
slice, holds a delete 5 s old that the author's record keeps.

**Steps.** As v1's, 1 to 4. In step 1 the author's clock is 5 s behind.

**Expected.**
1. S receives a put on `D` holding v1's content under a new run, and a
   delete of `D2` under the same run. The author's clock plays no part.
   G's GET of `D` draws a snapshot whose entry carries that put's stamp,
   and its GET of `D2` one whose entry is a tombstone. The target adopts
   the put: `Run` receives nothing, and the report's observed version
   moves to it, still `converged`, `in_force` `observed`.
2. The report is converged on v2, which the target acted on.
3. The author's new run is a 32-bit one where its run's record went
   with its documents' (core §4.3). S receives a put on `D` holding
   v2's content under the new run: adopted. The report moves to that version, still
   `converged`, `revision` 2. A GET of `D2` draws a snapshot with no
   entry: its delete was lost with the record.
4. The report is converged on v3.

## §7 Last-known from an archive, the author absent (§2.9)

As v1's. The archive's record of `D` keeps v1's envelope, so the target
acts on v1 under `s1`, `last_known` true. In step 2 the restarted author
puts v1's document again under a new run. The target adopts it, and its
report moves to that version with `last_known` false.

## §8 The kill switch (§2.12)

As v1's. `Rp` holds the baseline's report, and the author receives no
GET of `D`.

## §9 A tool's questions (§2.17, §5)

**Setup.** As v1's. T finds the target's report by `desired.report`
(§2.10), and GETs `D` from the author.

**Steps.** As v1's, 1 to 6, and:

7. After §15's step 1, T asks.

**Expected.** 1 to 5 as v1's.

6. Once: converged **not yet**, `behind`. Over the window, every reading
   is not yet on v6: T finds the target behind v6 for at least 2 s. A
   version has no time, so T finds nothing from one reading, whatever
   clock it trusts.
7. Converged: **unobservable**, `author_silent`. The author answers a
   snapshot with no entry under a run the report does not name. In
   force: **yes**, `desired`.

## §10 Last-known confirmed by its author; a present author's silence (§2.5, §2.9)

As v1's, with this reading of Expected 2. The target's seed carries the
held stamp `s1`, the archive's record's. The author, the same instance
throughout, answers with that stamp alone. That is its own word of the
version: **confirmed**. The target acts on nothing, and puts its report
again with `last_known` false.

## §11 A role on two documents refuses the start (§2.1, §2.3)

As v1's, with `tasking.v2` and `worker.v2`.

## §12 A `config.v1` or `config.v2` owner's re-mint is a restart (§2.7, §2.16)

As v1's, the author's contract listing `desired.v2`. The owner's report
is its own, in its view or its record.

## §13 A conditional re-read, and what a read waits (§2.5)

**Setup.** One router, R1. P plays the author. The target starts, and
its seed draws v1's snapshot from P, its entry at (R, 7). The target
converged on v1. Its period is 2 s.

**Steps.**
1. **Unchanged.** P answers each re-read with the held stamp it carried.
   The test waits 5 s.
2. **Changed.** P answers the next re-read with a snapshot of v2, its
   entry at (R, 9). It answers the one after as step 1 says.
3. **Not the one it sent.** P answers the next re-read with the held
   stamp (R, 8).
4. **An author that reads no payload.** P answers the next re-read with
   the snapshot of v2, its entry at (R, 9).
5. **One at a time.** P leaves the next re-read unanswered. 0.2 s after
   it arrives, P declares a second instance token at `lab/author`.
6. **Across a face.** The target stops. It starts again across a face
   whose policy states `reply_timeout_ms = 6000`
   ([`link.v2`](../link/v2.md) §2.20), with a period of 8 s. P answers
   each GET 4 s after it arrives: the seed whole, and each re-read with
   the held stamp it carried. The test waits 20 s.
7. **A period below the timeout.** The target stops. It is started
   across the same face with a period of 2 s. The test waits 5 s.

**Expected.**
1. Each re-read carries the 6 bytes `0xe0`, R and seq 7, under
   `zenoh/bytes`. The target acts on nothing, and `Rp` and `Run` receive
   nothing. An unchanged reply is 6 bytes of payload where v1's
   document was 44.
2. The target judges the entry: a higher seq of the held run. It acts on
   v2, and its report is converged on v2. The next re-read carries
   (R, 9).
3. A held stamp the target did not send is no answer. It acts on
   nothing, and its report is unchanged. Its next re-read carries (R, 9)
   again.
4. The same version: nothing is acted on.
5. P receives no second GET while the first is unanswered. The seed that
   fell due waits. When the first ends at its timeout, 1 s, the target
   makes one GET, the seed.
6. The seed draws v2's snapshot 4 s after it was asked, and the target
   converges on v2. A target that waited 1 s would have drawn nothing.
   P receives at most 3 GETs in the 20 s, and none while another is
   unanswered.
7. The target refuses to start. S sees no instance token of
   `lab/worker`.

## §14 The watch (§2.5)

**Setup.** `tasking.v2`, watched. The author on R1. The target is across
a face from it: no presence crosses, so the target never sees its
author's instance token appear. The author's heartbeat crosses, and the
test can cut the face and heal it. The target has no period: it watches.
`D2 = zk2/lab/author/tasking.v2/state/tasks/lab/spare` is a second slice
of the author's. The author holds v1 on `D`.

**Steps.**
1. The target starts. The test waits 10 s.
2. The author puts a document on `D2`. The test waits 6 s.
3. The author puts v2 on `D`. The test waits 6 s.
4. The test cuts the face. The author puts v3 on `D`. 5 s later the test
   heals the face, and waits 8 s.
5. The author's process ends without closing, and restarts from its
   record. The test waits 8 s.
6. The author's process is stopped for 6 s, past the horizon, and then
   let run again. The test waits 4 s.

**Expected.**
1. The target seeds and converges on v1. The first heartbeat it reads
   makes it read once, since it expects none. That read carries `s1`
   and is answered with it alone. From then on every heartbeat's section
   is the one it expects. The author receives no further GET of `D` in
   the 10 s, though 4 heartbeats or more crossed.
2. The next heartbeat's digest differs: another member changed. The
   target makes one conditional read, answered unchanged. `Rp` receives
   nothing. Then it reads nothing more.
3. The target hears v2 live, and converges on it. A target that moves
   what it expects makes no GET. One that does not makes one, at the
   next heartbeat. Either makes at most one.
4. The first heartbeat after the heal follows a gap in the seqs. The
   target reads, with `s2`. The author answers whole, and the target
   acts on v3. The next heartbeat makes it read once more, answered
   unchanged. Then it reads nothing more.
5. S receives a put on `D` holding v3's content under a new run. The
   target hears it and adopts it. The first heartbeat of the new
   instance is of another run. The target reads, and is answered
   unchanged. `Run` receives nothing.
6. While the author is stopped, no heartbeat crosses, and the document
   passes its horizon. The target keeps v3 in force. `Rp` and `Run`
   receive nothing, and the target makes no GET. When the author runs
   again, its next heartbeat is in sequence and unmoved: no GET either.
   At no step did a heartbeat alone move the report.

## §15 A delete past the window, then a restart (§2.8)

**Setup.** The author's tombstone window is 10 s. The author on R1 and
the target on R2, the link up. The target converged on v1, under the
author's run R.

**Steps.**
1. The test cuts the link. The author deletes `D`. 15 s later, past the
   window, the author's process ends and restarts from its record, which
   has dropped the delete (v1 §2.11). The test heals the link.
2. The author deletes `D` again: a delete it puts on the bus, though it
   holds no member there.

**Expected.**
1. The target sees the author's instance token appear, and seeds with
   the held stamp `s1`. The author's new instance writes under another
   run. It holds no member at `D`, so it answers a snapshot with no
   entry under that run. That is no verdict. The target keeps v1 in
   force, its report stays converged on v1, and `Run` holds v1's
   content. This is the residual §2.8 leaves: a target cut off across
   both a delete past the window and its author's restart.
2. The target hears a delete under the new run: another run, admitted.
   It reverts. The report is `reverted`, its observed version that
   delete's. A deployment heals the residual so, or by a terminal
   document (v1 §2.8).
