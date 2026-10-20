# `alarms.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §7 are the runtime's, and
the reference runs them as tests of `zenkey`
(`zenkey/tests/profile_alarms.rs`, one test per section, named after it;
#800, AB). §8 to §10 are a tool's, and the reference tool runs them as
tests of zenctl (`zenctl/tests/live_zk2.rs`, `alarms_s8_…` to
`alarms_s10_…`, T being `zenctl alarms` and, in §8, the watchdog's
`alert-firing`; #800, AC).

**Conventions.**
- **Section numbers** in parentheses (§2.5) are v1.md's rules; core
  sections say "core", and `freshness.v1`'s say so.
- **The bus.** R1 is a router with timestamping on, as a router's default
  is. Owners, the subscriber S, the GET reader G and the tool T are its
  clients. Timeouts are 1 s unless stated.
- **The contract** is the standard one, [`alarms.v1.toml`](alarms.v1.toml),
  with JSON payloads. **The owner** is `lab/svc`, implementing `alarms.v1`,
  and its keys are `zk2/lab/svc/alarms.v1/state/alarms/<alert_key>`.
- **The alarm A** is rule `link_down`, labels `{peer: r2, port: eth0}`,
  alert key `a659f813308ad1da` (§2.2), at severity `warning` unless a step
  says otherwise. "A firing" is A's document with `state: firing`.
- **S** subscribes to `zk2/lab/svc/alarms.v1/**` before the owner starts,
  and records each delivery's arrival on its monotonic clock, its key, its
  kind, its payload and its stamp. **G** GETs
  `zk2/lab/svc/alarms.v1/state/alarms/*` as core S4 says. "Reads" applies
  v1.md §2.6 to S's deliveries, and "judges" §2.14 to the reading at that
  instant, the descriptor read as core §3.3 says.
- **The clock.** G and T take the deployment's word for their clock
  (`freshness.v1` §2.6, ground 1): every session of a scenario runs on one
  host. §6 measures nothing else.
- **The horizon** is the contract's, 900 s, so the owner confirms each
  firing alarm at least every 450 s. §2 waits 455 s, §6 about 910 s and
  §10's step 2 listens 905 s, and a runner MAY keep them in a slow tier.
  The reference keeps them there, and its default run checks what §2's
  and §6's owner puts on a copy of the standard contract with
  `freshness.ttl_s = 4`, made in memory and never published (0.2), and
  §10's steps 1 and 3 (0.3).
- **The owner example** (0.3, F-133). The reference's owner,
  `zenkey/examples/owner.rs` run with `--alarms`, is the one a black-box
  runner drives, and `owner --help` is its agreed interface: its options,
  what it prints, the command word `alarm` and each command, the
  condition's JSON, every answer line, the record file's format, and that
  a copy of the standard contract with a short horizon is accepted as a
  contract file. §9's liar is no owner the text allows, so a runner writes
  it itself.
- **Jitter.** A gap between deliveries is measured at S. The reference
  allows 200 ms on loopback.

## §1 Raise (§2.2, §2.5, §2.6)

**Steps.**
1. The owner starts with nothing firing. G GETs.
2. The owner raises A. G GETs.
3. The owner raises A again, as a duplicate observation of the same
   condition, at the same severity.

**Expected.**
1. G's GET ends at its final reply with no value (core S2).
2. S receives one put on `…/state/alarms/a659f813308ad1da`, stamped by the
   owner (core S1): A firing, `timestamp` within 1 s of the step, no
   `observed_at_ms`. S reads it **raised**. G's reply holds the same
   document under the same stamp (core S2), and G judges **yes**,
   `firing`.
3. Nothing changes: the owner puts nothing, or the same document, which S
   reads **confirmed**. No second key appears: the key is the document's,
   whatever the order of its labels (§2.2).

## §2 Escalate, refresh and confirm (§2.5, §2.7)

**Steps.** A firing at `warning`, raised at instant *t₀*.
1. The owner escalates A to `critical`.
2. The owner refreshes A: its summary changes, and a label `host.id`
   is added.
3. The owner changes nothing for 455 s.

**Expected.**
1. S receives a put on the same key, `critical`, its `timestamp` later
   than *t₀*, no `observed_at_ms`, and reads it **escalated**.
2. S receives a put on the same key: the escalation's `timestamp`, kept,
   and `observed_at_ms` at least that `timestamp`. The alert key is
   unchanged: `host.id` is host-scoped (§2.2). S reads it **refreshed**.
3. S receives the refreshed document again, unchanged, `observed_at_ms`
   included, no two of its deliveries more than 450 s apart, under a later
   stamp of the same id (`freshness.v1` §2.4). S reads each **confirmed**.

## §3 Resolve, then delete (§2.5, §2.6)

**Steps.** A firing.
1. The owner resolves A.
2. G GETs within 1 s of the delete.
3. G GETs 65 s after it, with the owner's tombstone window at its 60 s
   default (core S3).

**Expected.**
1. S receives a put of A with `state: resolved`, a new `timestamp`, no
   `observed_at_ms`, then a delete of the key, within 60 s of the put and,
   for the reference, within 1 s. S reads **resolved**, then **retired**.
2. G's reply holds a `reply_del` for the key, with the delete's stamp
   (core S3). G judges **no**, `deleted`.
3. G's GET draws nothing for the key.

## §4 Re-raise (§2.5)

**Steps.** §3, then the owner raises A again.

**Expected.** S receives a put of A firing on the same key, its
`timestamp` later than the resolve's, and reads **raised**.

## §5 A restart mid-incident (§2.9)

**Setup.** The owner keeps a persistent record of the alarms it holds
(the reference: a file). It holds A firing at `critical`, raised at
*t₀*, and B, rule `disk_full`, labels `{mount: /var}`, firing. Its record
holds two more documents, written by a test before the restart:
- R: rule `fan_failed`, `state: resolved`, whose delete was lost;
- P: A's document stored under another key, a phantom
  (`…/state/alarms/0000000000000001`), whose alert key does not re-derive
  from it.

**Steps.**
1. The owner's process ends without closing. S listens 3 s.
2. While it is down, B's condition clears.
3. The owner restarts, and evaluates its rules once.
4. G GETs.

**Expected.**
1. S receives no delete and no resolved put (§2.9).
2. Nothing.
3. Before the new instance token appears (a SHOULD), and before any GET is
   answered with them (a MUST), S receives:
   - A firing, unchanged, `timestamp` *t₀* kept, under the new instance's
     stamp id. S reads it **confirmed**.
   - a delete of R's key, and of P's key. S reads each **retired**: it
     held neither (§2.6, 0.2).
   - B: a put, unchanged, which S reads **confirmed**; then, at its first
     evaluation, a resolved put and a delete, which S reads **resolved**,
     then **retired** (0.2).
4. G's reply holds A, stamped by the new instance, whose `meta.zid` is the
   stamp's id; G judges A **yes**, `firing`, never `previous_instance`.

## §6 A hung owner goes stale (§2.7, §2.14)

**Steps.** A firing, confirmed by the owner.
1. The owner stops evaluating A's rule, and so stops confirming A
   (the reference: its writer's re-puts turned off). It keeps its tokens
   and answers its GETs.
2. 905 s after S's last delivery of A, S judges, and G GETs and judges.

**Expected.**
1. S receives nothing more of A.
2. Presence: the owner's instance token is present. S judges §5's first
   question **unobservable**, `stale`, and its last **yes**, `stale`. G's
   reply holds A firing under a stamp more than 900.5 s old, and G judges
   the same. Neither reports A firing now, nor resolved (§2.7).

## §7 At the bound (§2.8)

**Setup.** The owner's descriptor states a cardinality of 2 for
`alarms/{alert_key}` (core §3.3).

**Steps.**
1. The owner raises A and B.
2. The owner raises C, rule `link_down`, labels `{peer: r3, port: eth1}`.
3. The owner resolves B, and raises C again.

**Expected.**
1. S receives A and B, firing.
2. The owner's writer is told the raise is refused (core §2.7). S receives
   no put of C, and no resolve or delete of A or B.
3. S receives B resolved and deleted, then C firing.

## §8 An absent owner, last-known, for a tool (§2.7, §2.9, §2.14)

**Setup.** An archive, `lab/archive` (core §4.4), records
`zk2/lab/svc/alarms.v1/state/**`. The owner holds A firing.

**Steps.**
1. T reads the owner and judges A.
2. The owner closes cleanly.
3. T reads presence, a complete read, then GETs the owner's alarms, then
   the archive's form of them (core §4.4), and judges A.

**Expected.**
1. **Yes**, `firing`.
2. S receives no resolved put and no delete of A (§2.9, a SHOULD NOT).
3. The owner's tokens are gone, and its GET draws no reply. T judges A
   **not asked**, `absent`, and shows the archived document, firing, as
   last-known, with its stamp, never as firing now (core S6). T's answer
   to "is anything firing at or above `warning`?" over `zk2/lab/*` counts
   the owner as not asked, and is not **yes** (§2.15). T knows of the
   owner from its step 1, so the owner is in scope though no token of it
   is found (§2.15, "the owners in scope", 0.2). A tool whose readings end
   with its process knows of it from the archive's keys instead (0.3).

## §9 A key that does not re-derive, for a tool (§2.2, §2.3, §5)

**Setup.** A test owner, `lab/liar`, puts on purpose:
- A's document on `…/state/alarms/0000000000000001`;
- a document with rule `link\ndown` and labels `{port: 22}` on its
  sanitized key, `…/state/alarms/16991d91fb818ca6`;
- A's document on its own key.

**Steps.** T reads `lab/liar`'s alarms and asks, for each key, whether it
re-derives from its document.

**Expected.** The first: **no**, `mismatch`, reported as a finding about
the owner. The second: **yes**, `sanitized`, which T MAY report as a
warning. The third: **yes**, `strict`.

## §10 Anything firing, for a tool (§2.15, §5)

**Setup.** Two owners: `lab/a` holds A firing at `warning`; `lab/b`
holds nothing, and has `alarms.v1` in its tokenless set (core §8.1).

**Steps.**
1. T subscribes to `zk2/lab/*/alarms.v1/state/alarms/*`, then GETs the
   same selector, and asks at once: is anything firing at or above
   `warning`? At or above `critical`?
2. T listens 905 s, losing no delivery, both owners present throughout,
   and asks again at `critical`. Its window counts as core §2.7 counts
   one, from the subscription's declaration plus the reply timeout, and
   its owners are watched a lease and the reply timeout past it (§2.15,
   0.3).
3. `lab/b` raises B at `critical`, and T asks at `critical`.

**Expected.**
1. At `warning`: **yes**, `any_firing`, 1. At `critical`: **unobservable**,
   `owner_unobservable`: `lab/a` is established (`answered`), and `lab/b`
   is not (`window_short`): its GET drew nothing, which alone is no
   verdict (core O5), and its window is not yet longer than 900 s. T finds
   `lab/b` by its descriptor, though it holds no `alive/alarms.v1/…`
   token (§2.13).
2. **No**, `none_firing`: `lab/b` is established by its window (`window`).
3. **Yes**, `any_firing`, 1.
