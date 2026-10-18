# `config.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

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
- **An idempotency key keeps refusals too**, and a key reused for another
  request is `invalid_request` (§2.14).
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
