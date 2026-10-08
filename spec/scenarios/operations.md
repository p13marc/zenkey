# Operation scenarios (core §5, §6)

Common setup: an owner `h1/tc` exposing operation `@op/interfaces/{if}/set`
(exclusive, fanout forbidden) and `@op/diagnostics` (fanout allowed).

## §1 At most once, while one instance serves (O1)

**Steps.**
1. 200 concrete calls with `BestMatching`, with one serving instance.
2. The same, with a second instance of the same service on another router
   (a split-brain).

**Expected.**
1. 200 executions.
2. 400 executions: one per side. This is the documented limit of O1, and §8
   below detects it.

*Spike S6: A 200, B 0 on one router; 200 and 200 across routers.*

## §2 Fan-out refusal (O2)

**Steps.** Call `zk2/h1/tc/tc.v1/@op/interfaces/*/set`, then `zk2/*/tc/…`.

**Expected.** One, then three `fanout_forbidden` refusals, and 0 executions.
Calling `@op/diagnostics` on `zk2/*/tc/…` with `All` + `None` gives one
reply per host.

## §3 Replies and errors (O3)

**Steps.** A call that succeeds; one with an invalid request; one to an
optional operation that is unavailable because a capability is missing.

**Expected.** A value reply on the concrete key. A `reply_err` envelope
with `code = invalid_request`. A `reply_err` envelope with
`code = unavailable` and `cause = capability`. The envelope's encoding
follows core §5.2.

## §4 Retries (O4)

**Expected.** A caller retries an idempotent operation after a timeout, and
never retries a non-idempotent one.

## §5 Silence (O5)

**Steps.** Call an operation that the access control refuses, then one whose
server is frozen.

**Expected.** Both return no reply. A tool reports "no answer", and
attributes it through presence. It never reports "no such operation".

## §6 Many replies (O6)

**Steps.** Two servers each reply 5 values and one summary to one call.

**Expected.** With consolidation `None`, 10 values and 2 summaries. With
`Latest` or `Auto`, values are lost: the reason `None` is required.

*Spike S6: `None` 10, `Latest` 1, `Auto` 1.*

## §7 Call metadata (O7)

**Steps.** A request carries the attachment `{actor, request_id}`.

**Expected.** The server sees both, and MAY record them. Nothing
authenticates them.

## §8 Split-brain diagnosis (core §6)

**Steps.**
1. Two instances of one service hold an interface token for one interface
   for longer than the grace period.
2. One instance re-mints (core §8.1).
3. A standby holds its instance token only.

**Expected.** A finding for 1. No finding for 2, whose overlap is shorter
than the grace period. No finding for 3. The runtime fences nothing.

*Spike S6: the token check found every split-brain and flagged no standby.*
