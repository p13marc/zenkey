# `alarms.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §8 and §10 restate
[`alarms.v1`'s](scenarios.md) where a record, a key or the confirmation
changes. §9, §11 and §12 are new. §1 to §7, §9 and §12 are the
runtime's. §8, §10 and §11 are a tool's.

No implementation runs them yet. The reference implements `alarms.v1`.

**Conventions.**
- **Section numbers** in parentheses (§2.5) are v2.md's rules. Core
  sections say "core", `freshness.v2`'s say so, and `v1 §n` is
  `alarms.v1`'s text.
- **The bus.** R1 is a router with timestamping on, as a router's
  default is. Owners, the subscriber S, the GET reader G and the tools
  are its clients. An owner's session runs with timestamping off (core
  §8.2). Timeouts are 1 s unless stated.
- **The contract** is the standard one, [`alarms.v2.toml`](alarms.v2.toml),
  with the packed records of [`alarms.v2.json`](alarms.v2.json).
- **The owner** is `lab/svc`. It implements `alarms.v2`, and a contract
  of its own, `tc.v1`, which lists `alarms.v2` in `uses` and declares on
  `[defaults]` the table `T1` of
  [`conformance/v2/tables.json`](conformance/v2/tables.json). Its rules
  are `hot` at index 0, `link_down` at 1, `sensor_lost` at 2 and
  `door_open` at 3. Its keys are
  `zk2/lab/svc/alarms.v2/state/alarms/<slot>` and
  `zk2/lab/svc/alarms.v2/state/words/<slot>`.
- **The alarms.**
  - **A** is `link_down` with `peer` `r2` and `port` `eth0`: rule 1,
    `labels` 2, alert key `a659f813308ad1da`.
  - **B** is `door_open`: rule 3, no label, alert key
    `abb089906038056a`.
  - **C** is `link_down` with `r3` and `eth1`: rule 1, `labels` 5, alert
    key `f3ba0819e7b57762`.
  - **H** is `hot`: rule 0, with an argument, alert key
    `335f24192ff5d0d4`.

  Each is at `warning` unless a step says otherwise. "A firing" is a
  record of A with `state` `firing`.
- **The epoch.** No owner is given a catalog, so the deployment epoch is
  0: a record's `at` is Unix seconds (core §7.6).
- **S** subscribes to `zk2/lab/svc/alarms.v2/**` before the owner
  starts, and to its heartbeat key covering `alarms.v2`,
  `zk2/lab/svc/freshness.v2/stream/heartbeat/alarms.v2`. It records each
  delivery's arrival on its monotonic clock, with its key, its kind, its
  payload and its in-band stamp. It reads each heartbeat as
  `freshness.v2` §2.5 says. **G** GETs
  `zk2/lab/svc/alarms.v2/state/alarms/**`, a collection read, as core S4
  says. "Reads" applies v2.md §2.6 to S's deliveries, and "judges" §2.14
  to the reading at that instant, the descriptor read as core §3.3 says.
- **The clock.** G and the tools take the deployment's word for their
  clock, as `alarms.v1`'s scenarios do: every session of a scenario runs
  on one host.
- **The horizon** is the contract's, 900 s. The owner's heartbeat
  confirms each firing alarm at least every 450 s. §2 and §6 wait it
  out. A runner MAY keep them in a slow tier, and run them on a copy of
  the standard contract with `freshness.ttl_s = 4`, made in memory and
  never published.
- **Sizes** are a put's payload under a 16-bit run. A 32-bit run adds 2
  bytes.

## §1 Raise, and both majors (§2.1, §2.2, §2.4, §2.5, §2.6)

**Steps.**
1. The owner starts with nothing firing. G GETs.
2. The owner raises A. G GETs.
3. The owner raises A again, as a duplicate observation of the same
   condition, at the same severity.
4. The owner raises B.
5. A second owner, `lab/old`, implements `alarms.v1`, its standard
   contract's revision 1.1, and holds A firing. A tool T reads the
   deployment: each instance's descriptor, one read over
   `zk2/lab/*/alarms.v2/state/alarms/**` and one over
   `zk2/lab/*/alarms.v1/state/alarms/*`. It asks whether anything fires
   at or above `warning`.

**Expected.**
1. G's GET is answered by the alarms' snapshot: one page of 16 bytes,
   with no entry, its collection's index 0, under the run the owner's
   descriptor states (core §3.3, §4.2).
2. S receives one put, on `…/state/alarms/0`: the lowest slot (§2.2).
   - Its payload is 18 bytes: CH `0x0b`, the run, the seq, and a record
     of 12 bytes.
   - The record reads `firing`, `warning`, rule 1, `at` within 1 s of
     the step, `labels` 2, `argument` `not_available`, and no `key`.
   - S receives nothing on `…/state/words/0`. S reads **raised**.
   - G's snapshot lists one entry, of id `0`, with the same record under
     the same in-band stamp, vouched for at the snapshot's time. G
     judges **yes**, `firing`.
3. Nothing is put (§2.5).
4. S receives one put on `…/state/alarms/1`, of 15 bytes: a record of 9
   bytes, rule 3, with no `labels`.
5. `lab/svc`'s descriptor lists `alarms.v2` and not `alarms.v1`, and
   `lab/old`'s the reverse (§2.1). T reads `lab/svc` by this text and
   `lab/old` by `alarms.v1`'s.
   - Its answer is **yes**, `any_firing`, 3: A and B of `lab/svc`, and
     A of `lab/old`.
   - The two A's have one alert key under two services. They are two
     alarms, by two references (§2.12).

## §2 Escalate, refresh and confirm (§2.5, §2.7)

**Steps.** H firing at `warning` on slot 0, raised at instant *t₀* with
a reading of 85.5: `argument` 1255.
1. The owner escalates H to `critical`.
2. The reading moves to 86.0, and the owner refreshes H.
3. The reading moves to 86.5 within 450 s of step 2.
4. The owner changes nothing for 905 s, and S judges.

**Expected.**
1. S receives a put on slot 0: `critical`, its `at` later than *t₀*. S
   reads **escalated**.
2. S receives a put on slot 0: the escalation's `at`, kept, and
   `argument` 1260. S reads **refreshed**.
3. S receives no put within 450 s of step 2's (§2.5, a SHOULD NOT). The
   owner MAY put the latest reading once they have passed.
4. S receives no put of H (`freshness.v2` §2.4). It receives a
   heartbeat at least every 450 s, of form 6.
   - Its tag is 7, that of `alarms.v2`'s revision 2.0, and its payload
     17 bytes.
   - Its section has a live count of 1, and slot 0's bit set.
   - At 905 s, past the horizon, S judges H **yes**, `firing`: the
     heartbeats confirmed it.

## §3 Resolve: one put, and no delete (§2.5, §2.6, §2.7)

**Steps.** A firing on slot 0.
1. The owner resolves A.
2. G GETs within 1 s.
3. G GETs 65 s later.
4. S reads the owner's next heartbeat.

**Expected.**
1. S receives one put on slot 0: `resolved`, `warning`, a new `at`,
   rule 1, `labels` 2. S reads **resolved**. No delete of the key
   follows, within 60 s or after it.
2. G's snapshot lists slot 0 as a live entry: the resolved record, with
   its last confirmation, its put's (confirmation 2, core §7.5). It
   lists no tombstone. G judges **no**, `resolved`.
3. The same answer. The resolved record is still the slot's value, with
   when it resolved. Under `alarms.v1` this GET drew nothing for A.
4. The heartbeat's section has a live count of 1, and slot 0's bit
   clear. It is in step with S's view.
   - S judges A **no**, `resolved`: never stale, whatever its age
     (§2.14).
   - Nothing fires, and the live count is 1 (§2.7).

## §4 Re-raise, and a resolved slot retaken (§2.2, §2.5, §2.6)

**Setup.** §3, A resolved on slot 0. A reader L holds A firing on slot
0, from a GET it made before §3's step 1. It does not subscribe.

**Steps.**
1. The owner raises B.
2. L GETs.
3. The owner raises A again.
4. The owner resolves B, then raises B again.

**Expected.**
1. S receives B firing on slot 0: the lowest slot whose record does not
   fire (§2.2). S held A resolved there. It reads A's record as
   `replaced`, with nothing to report, then B **raised**.
2. L's snapshot lists slot 0 with B's record, under the run of the
   record L holds. L held A firing there, so it reads A **resolved**,
   by `rebound`, then B **raised** (§2.6). It judges A **no**,
   `rebound`.
3. S receives A firing on slot 1, its `at` later than its resolve's. A
   holds no slot any more, so it takes the lowest free one. S reads
   **raised**.
4. S receives B resolved on slot 0, then B firing on slot 0: the slot
   its resolved record is on. No identity is on two slots at any step.

## §5 A restart mid-incident (§2.9)

**Setup.** The owner keeps a persistent record of the alarms it holds,
by slot (§2.9). It holds A firing at `critical` on slot 0, raised at
*t₀*, and B firing on slot 1. It holds D on slot 2, an alarm by words:
rule `disk_full`, labels `{mount: /var}`, alert key `719d159937e54eb8`.
A test wrote two more entries into its record before the restart:
- R, on slot 3: a resolved record of `sensor_lost`, sensor 7;
- P, on slot 4: A's record, stored with the alert key
  `0000000000000001`, which its words do not derive.

The new instance runs a later revision of `tc.v1`. Its table appends
the rule `disk_full`, with one label `mount` of values `/var` and
`/home`, at index 6. S and G hold both revisions.

**Steps.**
1. The owner's process ends without closing. S listens 3 s.
2. While it is down, B's condition clears.
3. The owner restarts. G GETs the moment S sees the new instance token.
   Then the owner evaluates its rules once.
4. G GETs.

**Expected.**
1. S receives no put and no delete (§2.9).
2. Nothing.
3. G's GET at the token is answered by a snapshot under the new
   instance's run, the one its descriptor states.
   - It lists slot 0, A firing, `at` *t₀* kept; slot 1, B firing; and
     slot 2, D firing. Each entry's stamp is under the new run: each
     was put again before the GET was answered.
   - D's record now has rule 6 and `labels` 0, and no `key`: the new
     table names its words (§2.9, "by name"). It derives the same alert
     key.
   - It lists neither slot 3 nor slot 4: nothing was put for R or P.
   - S receives, in no order against the token: A on slot 0, which it
     reads **confirmed**; B on slot 1, **confirmed**; and D on slot 2,
     the same identity with other fields, **refreshed**.
   - At the first evaluation S receives B resolved on slot 1, and reads
     **resolved**.
4. G judges A **yes**, `firing`, never `previous_instance`: its
   snapshot is under the run the descriptor states.

## §6 A hung owner goes stale (§2.7, §2.14)

**Steps.** A firing on slot 0, confirmed by the owner, and B resolved
on slot 1.
1. The owner stops evaluating A's rule, and so stops confirming A. It
   keeps its tokens, sends its heartbeats and answers its GETs.
2. 905 s after the last heartbeat that vouched for A, S judges. G GETs
   and judges.

**Expected.**
1. S receives no put of A. Each heartbeat has a live count of 2, and
   both bits clear.
2. The owner's instance token is present.
   - S judges A **unobservable**, `stale`, and §5's last question
     **yes**, `stale`. It reports A neither firing now nor resolved.
   - S judges B **no**, `resolved`. Its bit is clear as A's is, and it
     is not stale.
   - G's snapshot lists A with its last confirmation, more than 900 s
     old, and B with its own. G judges the same.
   - A reader that took the live count for two firing alarms, or A's
     clear bit for a resolve, would break §2.7.

## §7 At the bound (§2.2, §2.8)

**Setup.** The owner's descriptor states a cardinality of 2 for
`alarms/{slot}` (core §3.3).

**Steps.**
1. The owner raises A and B.
2. The owner raises C.
3. The owner resolves B, and raises C again.
4. The owner raises B again.

**Expected.**
1. S receives A firing on slot 0 and B firing on slot 1.
2. The owner's writer is told the raise is refused (core §2.7). S
   receives no put of C, no resolve of A or B, and no put on slot 0 or
   slot 1.
3. S receives B resolved on slot 1, then C firing on slot 1: a resolved
   record takes no room. S reads B's record as `replaced`.
4. The raise is refused: both slots fire.

## §8 An absent owner, last-known, for a tool (§2.7, §2.9, §2.14)

**Setup.** An archive, `lab/archive` (core §4.4), records
`zk2/lab/svc/alarms.v2/state/**`. The owner holds A firing on slot 0.

**Steps.**
1. T reads the owner and judges A.
2. The owner closes cleanly.
3. T reads presence, a complete read, then reads the owner's alarms,
   then the archive's form of them (core §4.4), and judges A.

**Expected.**
1. **Yes**, `firing`.
2. S receives no resolved put and no delete (§2.9, a SHOULD NOT).
3. The owner's tokens are gone, and its read draws no reply.
   - T judges A **not asked**, `absent`. It shows the archived record
     of slot 0, firing, as last-known, never as firing now (core S6).
   - It names the record by the table of the revision the owner's last
     descriptor named, which T holds from step 1. A tool without it
     shows the record as unnamed: `rule 1, labels 2`.
   - T's answer to "is anything firing at or above `warning`?" over
     `zk2/lab/*` counts the owner as not asked, and is not **yes**.

## §9 Words (§2.4, §4, §5)

**Setup.** `lab/svc` as above. A second owner, `lab/plain`, implements
`alarms.v2` and declares no table. T holds `tc.v1`'s revision, the one
`lab/svc`'s descriptor names. T2 holds an earlier revision of `tc.v1`
alone, whose table has `door_open` at index 1 and `link_down` at
index 3.

**Steps.**
1. `lab/plain` raises A, with the summary "Link to r2 is down".
2. `lab/plain` refreshes A: its summary becomes "Link to r2 flaps".
3. `lab/svc` raises A. It then raises W, rule `disk_full` with labels
   `{mount: /var}`, which its table does not hold.
4. T and T2 read both owners.
5. A test owner, `lab/liar`, puts on purpose a record by words with A's
   alert key on slot 0, and the words of `disk_full` on `words/0`. T
   asks whether the slot's words derive its record's key.

**Expected.**
1. A subscriber to `zk2/lab/plain/alarms.v2/**` receives two puts.
   - One is on `…/state/words/0`: an `alarms.v2.Words` document as
     strict JSON, after an envelope whose CH is `0x00`: a JSON type
     has no tag.
   - One is on `…/state/alarms/0`, of 23 bytes: rule `not_available`,
     no `labels`, and `key` `0xa659f813308ad1da`.
   - The words' seq is below the record's: the owner put them first
     (§2.4). The two ride two priorities, so they can arrive in either
     order.
2. The subscriber receives a put on `…/state/words/0`, and none on
   `…/state/alarms/0`. The new words derive the record's key, so it
   takes them. A's state and its `at` do not move.
3. `lab/svc`'s A is one put of 18 bytes on its slot 0, rule 1 and
   `labels` 2, with no words. Its W is words on `words/1`, then a
   record by words on `alarms/1`.
4. T shows `lab/svc`'s A as `link_down`, with `peer` `r2` and `port`
   `eth0`, and the summary "Link to r2 on eth0 is down".
   - T shows `lab/plain`'s A by its words, "Link to r2 flaps". Both A's
     have the alert key `a659f813308ad1da`.
   - T2 does not hold the revision `lab/svc`'s descriptor names. It
     shows that A as `rule 1, labels 2`, unnamed, never as `door_open`,
     the name its own revision has at that index.
   - T2 shows `lab/plain`'s A and `lab/svc`'s W with their words: words
     need no table.
   - Each judges every alarm **yes**, `firing`. A name plays no part in
     the answer.
5. **No**, `mismatch`. T shows the alarm by its alert key, with no
   words.

## §10 Anything firing, for a tool: one snapshot (§2.15, §5)

**Setup.** Two owners. `lab/a` holds A, B and C firing at `warning`, on
slots 0 to 2. `lab/b` holds nothing, and has `alarms.v2` in its
tokenless set (core §8.1).

**Steps.**
1. T subscribes to `zk2/lab/*/alarms.v2/state/alarms/*`, then reads
   `zk2/lab/*/alarms.v2/state/alarms/**`, and asks at once: is anything
   firing at or above `warning`? At or above `critical`?
2. `lab/b` raises B at `critical`, and T asks at `critical`.
3. `lab/b`'s process restarts with no record. T asks at `critical`
   before it reads again, then reads again and asks.

**Expected.**
1. At `warning`: **yes**, `any_firing`, 3. At `critical`: **no**,
   `none_firing`. Both owners are established, by `snapshot`, with no
   window to wait out.
   - `lab/a`'s answer is one page of 70 bytes, with three entries: a
     late joiner's whole seed, in one reply. Two of its records carry
     labels, at 12 bytes. With three records of 9 bytes it is the 64
     bytes that v2.md's Appendix B prices at one SDU on zenoh-modem's RF
     face. A runner checks the page's size alone.
   - `lab/b`'s answer is one page of 16 bytes, with no entry: its word
     that nothing fires (core §4.2).
   - T finds `lab/b` by its descriptor, though it holds no
     `alive/alarms.v2/…` token (§2.13).
2. **Yes**, `any_firing`, 1.
3. Before the read: **unobservable**, `owner_unobservable`. `lab/b` had
   a gap in its presence since T's read (`owner_gap`).
   - After it: **no**, `none_firing`. The new instance's snapshot has no
     entry, under the run its descriptor states.
   - The B that T held from the earlier instance is left out, and reads
     **no**, `not_held`.

## §11 A reader built for `alarms.v1`, beside an `alarms.v2` owner (§2.1, §2.17)

**Setup.** `lab/svc` implements `alarms.v2`, and holds A firing at
`critical`. `lab/old` implements `alarms.v1`, and holds one alarm firing
at `info`. T1 is a tool that follows `alarms.v1`'s text alone, as one
built before this major does. T2 follows this text.

**Steps.**
1. T1 asks whether anything fires at or above `warning`, over
   `zk2/lab/*/alarms.v1/state/alarms/*`.
2. T1 asks the same over `zk2/lab/svc/alarms.v1/state/alarms/*`.
3. T2 asks step 1's question, over step 1's selector.

**Expected.**
1. T1 reads `lab/svc`'s descriptor, which lists no `alarms.v1`. It
   answers of that owner **not asked**, `not_listed` (v1 §2.14).
   - `lab/old` is established, by its GET's answer.
   - T1's answer is **no**, `none_firing`, with one owner established
     and one not asked, while `lab/svc` fires at `critical`.
   - T1 conforms to `alarms.v1`, and is wrong about the deployment.
     That is the hazard of §2.17. Its count of owners not asked is the
     only trace.
   - A runner checks that T1 reports `lab/svc` as not asked, and never
     among the owners it found with nothing firing.
2. **Not asked**, `no_owner`: never "nothing fires".
3. **Yes**, `any_firing`, 1. T2 reads both majors of the owners the
   selector selects, whichever major it spells (§2.17). It reports A by
   its reference: `lab`, `svc`, `a659f813308ad1da`.

## §12 A service moves from `alarms.v1` (§2.5, §2.9, §2.16)

**Setup.** `lab/svc` implements `alarms.v1`, revision 1.1, with `tc.v1`
as above. It holds A firing at `critical`, its `timestamp` *t₀*, an
instant with 123 ms past its second. Its record holds A's document. An
acknowledgement of A was made against *t₀*, by the reference `lab`,
`svc`, `a659f813308ad1da`. A notifier N, which follows this text,
reported A firing.

**Steps.**
1. The service restarts as an instance that implements `alarms.v2`, and
   adopts from its record.
2. T reads the owner. N goes on reading.

**Expected.**
1. The new descriptor lists `alarms.v2` and not `alarms.v1`.
   - S receives A firing on `…/alarms.v2/state/alarms/0`: `critical`,
     rule 1, `labels` 2, and `at` equal to *t₀*'s second (§2.9).
   - The new instance puts and deletes nothing on an `alarms.v1` key.
2. T judges A, by its reference, **yes**, `firing`.
   - The acknowledgement still applies: A's `at` is at most *t₀*
     (§2.5).
   - N reports nothing. The reference, the severity and the answer are
     those it told (§2.16).
