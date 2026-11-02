# `config.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. They are
[`scenarios.md`](scenarios.md)'s fourteen sections, on `config.v2`'s
wire. §1 to §12 are the runtime's, and run as tests of `zenkey`, one test
per section, named after it. §13 and §14 are a deployment's and a tool's,
and run as tests of the tool that reads `config.v2`. They are `zenctl`'s
`config_v2_scenario_13_…` and `config_v2_scenario_14_…`.

**Conventions.**
- **Section numbers** in parentheses (§2.9) are v2.md's rules; `v1 §n`
  are `config.v1`'s, and core sections say "core".
- **The bus.** R1 is a router with timestamping on. The owner, the caller
  C, the subscriber S and the tool T are its clients. Timeouts are 1 s,
  except a call's, which is 2 s, unless stated. No catalog is in force,
  so the deployment epoch is 0 (core §8.6).
- **The contract** is the test contract,
  [`conformance/contracts/rfcfg.v1.toml`](conformance/contracts/rfcfg.v1.toml),
  with its packed file and the standard schema.
- **The owner** is `lab/rf0`, implementing `rfcfg.v1` with every resource
  exposed, a persisted layer, and the startup values of
  [`conformance/v2/lifecycle.json`](conformance/v2/lifecycle.json):
  - `duty` (hot): `fraction_ppm` 10000 from its file, `period_s` 3600 and
    `enforce` true by default;
  - `radio` (reach): `frequency_hz` 868100000 from its file;
  - `link` (reach): `net_id` 1 by default;
  - `sim` (hot): `pin` 0 from its file, sensitive, and `attempts` 3;
  - `queue` (hot, outside the allowlist): `tx_queue_len` 100;
  - `modulation` (contract): `spreading_factor` 9 from its file.

  Its floor is 10 s, and its device states `apply_at` 1 s after it
  receives a reach `set`. Its device is simulated: a test sets each act's
  result, and reads what the device holds.
- **Keys.** `Cfg` is `zk2/lab/rf0/rfcfg.v1/state/config`, `Op(x)` its
  `@op/x`, and `Ev` its `events/change/*`.
- **S** subscribes to `zk2/lab/rf0/rfcfg.v1/**` and `zk2/lab/rf0/@zk/**`
  before the owner starts, and to its heartbeat key,
  `zk2/lab/rf0/freshness.v2/stream/heartbeat/rfcfg.v1`. It records each
  delivery's arrival, key, kind, payload and stamp. It reads each record
  as v2.md §3 lays it out, and each heartbeat as `freshness.v2` §2.5 says.
- **C** writes each request as its `Set<T>` or `Control` record in a
  query (core §7.5), and calls as core O1 says, with the call metadata
  `{"actor": "ops", "request_id": …}` (core O7). It reads each reply as
  the value form or the error form, and GETs as core S4 says.
- **"The fixture's step"** names a step of `lifecycle.json` whose puts,
  replies and events the scenario observes on the bus.

## §1 A hot set, read back, and a dry run (§2.3, §2.9, §2.17)

**Steps.**
1. The owner starts. T, holding a liveliness subscriber on
   `zk2/lab/*/@zk/**` with history, GETs `Cfg` the moment it sees the
   owner's instance token.
2. C calls `Op(set/duty)` with `fraction_ppm` 20000.
3. C GETs `Cfg`.
4. C calls `Op(set/duty)` with `period_s` 60, as a dry run.
5. The owner puts nothing for 65 s, and S judges `Cfg`.

**Expected.**
1. T's GET answers the record: revision 0, nothing pending, no last
   change, `floor_s` 10, every settable group's `writable` bit but
   queue's, `pin` with its source alone. It was put before the tokens
   (§2.3).
2. The reply is the value form: `fraction_ppm` 20000 at `runtime`,
   `differs` set, revision 1, a last token, duty's last bit. Its seq is
   the seq of the put S received just before it (§2.9). Then S receives
   one event on `Ev`, `applied`, with that token and the edit 10000 →
   20000, `actor` `"ops"`, its seq one past the put's.
3. `Cfg` answers the reply's record, byte for byte.
4. The reply's record has `would_be` set and `period_s` 60 at `runtime`.
   Its seq is step 2's event's, the last minted. S receives no put, and C
   still holds step 3's record as the owner's (§2.9).
5. S receives no put or delete of `Cfg` (§2.3). It receives a heartbeat
   at least every 30 s, which vouches for it. At 65 s, past the horizon,
   S judges it fresh.

## §2 A reach set, confirmed (§2.8, §2.9, §2.10)

**Steps.**
1. C calls `Op(set/radio)` with `frequency_hz` 869525000 and `confirm_s`
   120, and notes the reply's arrival.
2. C calls `Op(confirm)` with the token, before `apply_at`.
3. C waits until a record it receives shows the change applied, then
   calls `Op(confirm)` with the token.
4. C calls `Op(confirm)` with the token again.

**Expected.**
1. The reply is the record: the change pending, radio's pending bit set,
   `pending_apply_at` and `pending_deadline` stated in whole seconds, 1 s
   and 120 s after the call's receipt, rounded up. It arrives before the
   device holds the new frequency. S receives the record put before the
   reply, then a second one with no `pending_apply_at` and `frequency_hz`
   869525000.
2. The error form, `busy`, under the first put's seq: not yet applied
   (§2.9).
3. The record: nothing pending, the last token the change's. S receives
   one event, `confirmed`, after the reply, with `actor` `"ops"` and the
   `request_id` of step 1's call (v1 §2.17).
4. The error form, `not_found`.

## §3 A reach set rolled back at its deadline (§2.8, §2.11, §2.17)

**Steps.**
1. 300 ms into a second, C calls `Op(set/radio)` with `confirm_s` 5: the
   window is the floor, 10 s.
2. C does nothing. 11.5 s after the call, C GETs `Cfg`, and calls
   `Op(confirm)` with the token.
3. A second owner, `lab/rf1`, configured as `lab/rf0`, takes the same
   `set`, and its simulated device refuses the apply at `apply_at`.

**Expected.**
1. The record's deadline is the next whole second after the call's
   receipt plus 10 s: 10.7 s after it, never 10.3 s.
2. The device sees the undo at that instant, within 0.1 s, and not
   before. Then S receives the record, `frequency_hz` 868100000 again at
   `file`, revision 2, then one event, `rolled-back`. The confirm is
   `not_found`. The device saw exactly one undo.
3. `lab/rf1` publishes one event, `rolled-back`, at `apply_at`, with no
   undo made: nothing was applied (v1 §2.11).

## §4 An undo that fails (§2.11)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 10. The test makes the
   device's undo fail.
2. 20 s after the call, C GETs `Cfg`.

**Expected.**
1. At the deadline, S receives the record, then one event, `partial`.
2. The record shows `frequency_hz` as the test's device reports it, or
   with neither source nor value when the device reports none; nothing
   pending. The device saw exactly one undo, and no second within the
   10 s after it.

## §5 A join, and a second writer (§2.7)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 120.
2. A second caller, C2, calls `Op(set/link)` with `confirm_s` 60.
3. C calls `Op(set/link)` with `net_id` 7 and the token, then
   `Op(set/duty)` with `fraction_ppm` 20000 and the token.
4. C confirms once a record shows the change applied.

**Expected.**
1. The record, pending.
2. The error form, `busy`. C2 reads the token from the record it GETs.
3. Two records: the same token and deadline, link's `apply_at`; then
   duty, radio and link's pending bits set, in the contract's order.
4. One event, `confirmed`, its edits duty's, radio's, then link's: the
   contract's order, not the join's (§2.17). No event was published at
   either join.

## §6 An extend (§2.10)

**Steps.**
1. C calls `Op(set/duty)` with `fraction_ppm` 20000 and `confirm_s` 30.
2. 20.4 s later, C calls `Op(extend)` with `confirm_s` 60.
3. C does nothing more.

**Expected.**
1. The record, the change applied and pending, deadline 30 s after the
   call.
2. The record, its deadline 81 s after step 1's call: the extend's
   receipt plus 60 s, rounded up. No event.
3. The undo runs at the new deadline, not the first: S receives the
   `rolled-back` event 81 s after step 1, ± 0.5 s.

## §7 Tokens, and a restart during a pending change (§2.6, §2.12)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 120, and waits until a record
   shows it applied.
2. The owner's process stops, and starts again.
3. C calls `Op(confirm)` with the token.
4. The owner restarts twice more. After each start, C calls
   `Op(set/duty)`, and notes the token's counter, its low 16 bits.
5. A test hook starts the owner's token counter at 65,534. C calls
   `Op(set/duty)` three times.

**Expected.**
1. The record, pending; the device holds 869525000. The token's high 32
   bits are the record's `instance`.
2. S sees the old instance token withdrawn and a new one declared (core
   §8.1). The new record, put before its tokens, shows another instance
   number, revision 0, nothing pending, no last change, and
   `frequency_hz` 868100000 at `file`; the device holds the startup
   frequency. No event is published for the old change.
3. The error form, `not_found`. The token is not one the new instance
   minted, whatever it mints next (§2.6).
4. The three counters are not all equal: each run's counter starts at
   random (§2.6).
5. The first two tokens share a run, with counters 65,534 and 65,535.
   The third's run is another. S sees the owner's descriptor state the
   new run, and the third reply's run is it. Every record keeps the same
   `instance`.

## §8 Persist, granted and not (§2.13, §2.21)

**Setup.** R1 runs access control with `default_permission: deny`. The
principal `ops` is granted `Op(set/duty)`, `Op(set/radio)`,
`Op(confirm)`, `Op(cancel)`, `Op(extend)` and the GET of `Cfg`; the
principal `admin` those and `Op(persist)`. A second owner, `lab/rf2`, has
no persisted layer.

**Steps.**
1. As `admin`, C calls `Op(set/duty)` with `fraction_ppm` 20000, then
   `Op(persist)` with the reply's `last_token`.
2. As `ops`, C calls `Op(set/duty)` with 30000, then `Op(persist)` with
   its token.
3. As `admin`, C calls `Op(persist)` on `lab/rf2`, after a `set` there,
   and GETs `lab/rf2`'s descriptor.
4. The owner restarts. C GETs `Cfg`.

**Expected.**
1. Both answered with the record; the persist's shows `fraction_ppm` at
   `overlay`, `differs` clear. No event for the persist.
2. The `set` is answered; the `persist` draws nothing before its timeout:
   a refusal by access control is silent (core O5), and C reports it so,
   never as `not_found`.
3. The error form, `unavailable`, cause `config`; the descriptor lists
   `persist` in `unavailable` with cause `config` (core §3.3).
4. `fraction_ppm` is 20000 at `overlay`: step 1's change survived; step
   2's did not.

## §9 A group outside the allowlist (§2.16)

**Steps.**
1. C GETs `Cfg`.
2. C calls `Op(set/queue)` with `tx_queue_len` 10.
3. C calls `Op(set/modulation)`.

**Expected.**
1. Every settable group but `queue` has its `writable` bit set.
2. The error form, `unavailable`, cause `config`. Nothing changed; no
   event.
3. Nothing, before the timeout: modulation is a contract group, with no
   `set`, so no queryable answers (§2.4). C reports silence.

## §10 An idempotent retry (§2.14)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 60 and `idempotency_key`
   1001. The test drops the reply before it reaches C.
2. C calls again, the same request, 5 s later, after the apply.
3. C calls with the same key and another `frequency_hz`.
4. C cancels the change, and 300 s after step 1's call, calls step 1's
   request again.

**Expected.**
1. The owner arms the change; C receives nothing. S receives its record.
2. The reply is step 1's, byte for byte: its run and seq are the first
   put's, and its record shows the change pending and not applied, though
   S has received the applied record since. The device saw one apply, and
   the owner published nothing new.
3. The error form, `invalid_request`, under the current mark.
4. A new pending record, with a new token: the key was forgotten at
   300 s.

## §11 Sensitive values (§2.15)

**Steps.**
1. C calls `Op(set/sim)` with `pin` 1234, then with `pin` 9999.
2. The test makes the device refuse a third `set`, of `pin` and
   `attempts`, with words holding the PIN.
3. C GETs `Cfg`, and calls `Op(persist)` with the first change's token,
   then with the second's.
4. The test reads the owner's log.

**Expected.**
1. Both applied. The records S receives after each differ only in their
   revision and last token: no bit depends on the PIN. Each event's edit
   is `{"group": "sim", "parameter": "pin", "redacted": true}`.
2. The error form, `app`, an `Error` detail `device_refused`, with no
   message.
3. The record shows `pin` with source `runtime` and nothing more. The
   first persist is `not_found`, the last change being the second. The
   second is `forbidden`.
4. No line of it holds `1234` or `9999`; a logged request writes
   `"<redacted>"`. No delivery S recorded, and no reply C received, holds
   either.

## §12 Desired state refused (§2.18)

**Setup.** A deployment binds `lab/rf0` to a desired document, put by an
author service `lab/author` as its own state.

**Steps.**
1. The author puts `{"revision": 1, "values": {"duty": {"fraction_ppm":
   20000}}}`.
2. The author puts `{"revision": 2, "values": {"duty": {"fraction_ppm":
   30000}, "radio": {"frequency_hz": 869525000}}}`.
3. The author puts `{"revision": 3, "values": {"sim": {"pin": 1234}}}`.

**Expected.**
1. S receives the record, `fraction_ppm` 20000, `desired` `converged`,
   `desired_revision` 1, then one event, `applied`.
2. No act and no event. The record's `desired` is `refused`, at revision
   2, `desired_code` `reach`, `desired_group` 1, radio's index, with no
   parameter index. `fraction_ppm` is still 20000: a document is refused
   whole.
3. `desired_code` `sensitive`, `desired_group` 3 and
   `desired_parameter` 0, sim's pin, and no act.

## §13 The record across a constrained face (§2.12, §2.19, §2.21)

**Setup.** `spec/scenarios/constrained.md` §1's shape: the owner behind
R1, and the caller C as a client of R1 across a constrained face, with the
`@zk` deny on it and a 1 KB batch. The face lets C's subscriptions to
`Cfg` and to the owner's heartbeat key cross, and its calls to
`Op(set/radio)`, `Op(confirm)`, `Op(cancel)` and `Op(extend)`, and
nothing else of the service. C binds statically:
it reads no presence and no descriptor, and holds the contract.

**Steps.**
1. C subscribes to `Cfg`, and GETs it once.
2. C calls `Op(set/radio)` with `confirm_s` 120, an `expected_revision`
   and an `idempotency_key`.
3. Once the push shows the change applied, C calls `Op(confirm)`.
4. The owner restarts. C reads the next push.

**Expected.**
1. The record, with `floor_s`: C sizes its window from it. No `@zk`
   delivery crosses the face.
2. The request is at most 16 bytes of payload, and the reply at most 64,
   the record pending. The applied record arrives by the push: C makes no
   GET after the reply.
3. The record, at most 64 bytes, nothing pending, the last token the
   change's.
4. Its `instance` differs from step 1's: C learns of the restart without
   presence (§2.12).

## §14 A tool's answers (§5)

**Setup.** The tool T holds the contract. It reads `lab/rf0`, and a
service `lab/plain` whose descriptor lists no `config.v2`.

**Steps.**
1. T asks "is a change pending?" of both, with the owner idle.
2. C arms a reach change; T asks again of `lab/rf0`, and asks "did this
   change land?" with its token.
3. The change rolls back at its deadline; T asks "did it land?" and "was
   its undo clean?".
4. C sets `duty` without a window; T asks "will `fraction_ppm` survive a
   restart?" and "is `queue` writable?".
5. The owner stops; T asks "is a change pending?" within 60 s, then
   after 70 s from the owner's last put.

**Expected.**
1. `lab/rf0`: no. `lab/plain`: not asked, its descriptor not listing
   `config.v2`.
2. Yes, with the token and the deadline; "did it land?": not yet.
3. No, `rolled-back`; and yes, clean.
4. No: `fraction_ppm`'s `differs` is set. And no: queue's `writable` bit
   is clear, though the contract declares it hot.
5. Within 60 s: not asked, the owner absent (presence's word). After it,
   with a reader that does not read presence: unobservable, the record
   stale; never no (v1 §5, core O5).
