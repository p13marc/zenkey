# Operation scenarios (core §5, §6)

Common setup: three hosts `h1`, `h2`, `h3`, each with a service `tc`
exposing `@op/interfaces/{if}/set` (exclusive, fanout forbidden) and
`@op/diagnostics` (fanout allowed).

## §1 At most once, while one instance serves (O1)

**Steps.**
1. 200 concrete calls with `BestMatching`, with one serving instance,
   through a session whose outgoing queries the test captures.
2. The same, with a second instance of the same service on another router
   (a split-brain).

**Expected.**
1. 200 executions. Each query carries target `BestMatching` and
   consolidation `None`, and the caller returns on the first reply, before
   the query completes.
2. 400 executions: one per side. With consolidation `None`, the caller sees
   both replies of each call; under `Latest`, one per key would vanish.
   This is the documented limit of O1, and §8 below detects it.

*Spike S6: A 200, B 0 on one router; 200 and 200 across routers.*

## §2 Fan-out (O2, O3)

**Setup.** Each `tc` also exposes `@op/interfaces/{if}/reset` (fanout
allowed, `replies = "one"`), with members `eth0` and `eth1`: on `h1` a
queryable per member, on `h2` one queryable over the template, and on `h3`
one queryable over the template that refuses every call with `busy`.

**Steps.**
1. Call `zk2/h1/tc/tc.v1/@op/interfaces/*/set`, then `zk2/*/tc/…`.
2. Call `@op/diagnostics` on `zk2/*/tc/…` with `All` + `None`.
3. Call `zk2/*/tc/tc.v1/@op/interfaces/*/reset` with `All` + `None`.

**Expected.**
1. One, then three `fanout_forbidden` refusals, and 0 executions.
2. One reply per host.
3. From `h1`, two value replies, on `…/interfaces/eth0/reset` and
   `…/interfaces/eth1/reset`. From `h2`, one value reply, on the key of the
   member its server named, which the call selected. One `busy` envelope,
   which the caller reports unattributed: a `reply_err` carries no key.
   Presence shows `h3` holding the token and sending no value, so `h3`
   refused or was silent, which the caller cannot tell apart.

## §3 Replies and errors (O3, §5.2)

**Steps.**
1. A call that succeeds.
2. One whose request does not decode as the request type.
3. One to `zk2/h1/tc/tc.v1/@op/interfaces/ETH0/set`, whose parameter chunk
   is not a canonical slug.
4. One to an optional operation that is unavailable because a capability
   is missing.
5. One whose handler returns without replying.
6. One refused with `app`: by an operation with no `error` type, by one
   with a JSON Schema `error` type, and by one with a raw `error` type.
7. **Beside a replica.** In a variant of `tc.v1` whose `set` is optional
   and whose `diagnostics` is replicated, a second instance of `h1/tc`
   exposes `diagnostics` only, and lists `set` `unavailable` (cause
   `config`); the first instance serves both. 200 concrete calls to `set`,
   both instances on one router.

**Expected.**
1. A value reply on the concrete key.
2. A `reply_err` envelope with `code = invalid_request`.
3. `invalid_request`, not `not_found`: the key names no member.
4. A `reply_err` envelope with `code = unavailable` and
   `cause = capability`.
5. A `reply_err` envelope with `code = internal`, never silence.
6. `app` with no detail; `app` with the error value inline, or none; `app`
   in a JSON envelope with the error's bytes as base64 text, or none. A
   detail that does not fit its envelope goes out as `internal` instead.
7. 200 executions on the first instance, and no `unavailable`: the replica
   declares no queryable on `set`.

The envelope's encoding follows core §5.2.

## §4 Retries (O4)

**Steps.** With retries configured, call an idempotent operation whose
server is frozen; then one whose server answers `busy`; then a
non-idempotent operation whose server is frozen. Then the idempotent one
again, with no retries configured.

**Expected.** The first is called again after each timeout, up to the
configured number of attempts, and ends silent. The second is called once
and returns `busy`: an envelope is an answer. The third is called once. The
last is called once: retrying is opt-in.

## §5 Silence (O5)

**Steps.** Call an operation that the access control refuses, then one whose
server is frozen.

**Expected.** Both return no reply. A tool reports "no answer", and
attributes it through presence. It never reports "no such operation".

## §6 Many replies (O6)

**Steps.**
1. Two servers each reply 5 values and one summary to one call.
2. A server sends 5 values, and its handler returns before its summary.
3. A replicated operation's two replicas sit behind two routers, and each
   replies 5 values and its summary to one concrete call.

**Expected.**
1. With consolidation `None`, 10 values and 2 summaries. With `Latest` or
   `Auto`, values are lost: the reason `None` is required.
2. The 5 values, then an `internal` envelope. The caller reports the
   replier as possibly partial.
3. 10 values and 2 summaries on one key. The caller reports that replier
   as possibly partial too: two instances on one key cannot be told
   apart.

*Spike S6: `None` 10, `Latest` 1, `Auto` 1.*

## §7 Call metadata (O7)

**Steps.** A request carries the attachment `{actor, request_id}`; then
`{"actor": "a", "extra": 1}`; then `{"actor": 3}`, `[]` and bytes that are
not JSON.

**Expected.** The server sees both members of the first, and MAY record
them. The second is metadata with an actor and no request id. The rest are
no metadata, and each call is served all the same. Nothing authenticates
any of them.

## §8 Split-brain diagnosis (core §6)

**Steps.**
1. Two instances of one service hold an interface token for one interface
   for longer than the grace period.
2. One instance re-mints (core §8.1).
3. A standby holds its instance token only.
4. Two instances of one service expose an interface whose only resources
   are replicated operations.
5. One instance exposes an interface's exclusive resources and its
   replicated operation; a second exposes the replicated operation only,
   and lists the exclusive optional resources `unavailable`.

**Expected.** A finding for 1. No finding for 2, whose overlap is shorter
than the grace period. No finding for 3. No finding for 4 or 5, which the
tool decides from the contract and the holders' descriptors: at most one
holder exposes an exclusive resource. The runtime fences nothing.

The tool reads presence twice, `grace` apart; a holder seen in one read
only is not part of a finding.

*Spike S6: the token check found every split-brain and flagged no standby.
Steps 4 and 5 are new in core 0.7: the reference's check, which compares
tokens only, reports both, which is the runtime change 0.7 records.*
