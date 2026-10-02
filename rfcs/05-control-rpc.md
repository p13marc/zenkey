# 05 — Control Plane: `@rpc`

**Status: v1.2 (ratified)** · normative chapter · *amended in v1.2, v1.25, v1.31, v1.38, v1.40, v1.42, v1.47 and v1.48 — see [CHANGELOG.md](CHANGELOG.md)*

All interaction — questions, instructions, downloads-of-detail — happens on
the `@rpc` plane through **queryables** (request/reply), never through
pub/sub commands. This chapter defines the procedure grammar, targeting,
parameters, reply conventions, and how the incumbent command/query channels
map onto it.

---

## 1. Key shape

```
<base>/v1/<origin>/@rpc/<producer>/<procedure...>
<base>/v1/@<service>/@rpc/<procedure...>
```

- `@rpc` is a verbatim chunk: no `*`/`**` data selector can ever reach a
  procedure, and no RPC traffic ever leaks into a data subscription
  (design property D2, [03-grammar.md §4](03-grammar.md)).
- `<procedure...>` is one or more chunks, registry-governed
  ([08-registry.md](08-registry.md)). Multi-chunk procedures group related
  endpoints (`artifact/request`, `artifact/status`).
- The **server** is the producer (it declares the queryable); the **client**
  is whoever GETs. Parameters ride the Zenoh selector
  (`?state=established;port=443`), request bodies ride the query payload.

## 2. Targeting — the fan-out problem, solved by position

The origin chunk makes addressing a property of the key, in both
directions:

| Intent | Selector |
|---|---|
| ask/instruct **one host** | `GET <base>/v1/h-xxx/@rpc/netlink/sockets?ip=10.0.0.7` |
| ask **the fleet**, collect all replies | `GET <base>/v1/*/@rpc/netlink/sockets?ip=10.0.0.7` |
| ask every producer of one origin | `GET <base>/v1/h-xxx/@rpc/*/health-detail` (if registered by several) |

This subsumes both control patterns the incumbent keyspace had to choose
between per channel:

- **Point control** (restart a unit, open a stream, run a capture) targets
  one origin structurally — a mistargeted instruction is now a *non-matching
  key*, not a payload filter every sensor must implement correctly.
- **Fan-in joins** (which host owns the socket for this flow?) keep their
  one-GET ergonomics via the `*` origin. Only hosts that serve the
  procedure reply; the client collects and joins, exactly as before.

Consequently the convention has **no `target` field in any envelope**:
addressing is never payload.

### 2.1 Fan-in call discipline (normative)

Zenoh's *defaults* do not deliver "collect all replies"; the following
discipline does, and fleet callers MUST follow it:

- **Target.** Fan-in GETs MUST set query target **All**. The default
  (`BestMatching`) short-circuits to a *single* queryable the moment any
  matching queryable is declared `complete` — one storage config away from
  silently collapsing the fleet to one reply. For the same reason, `@rpc`
  queryables MUST NOT be declared `complete`.
- **Reply key.** A procedure MUST reply on its **own concrete key**
  (`…/h-xxx/@rpc/<producer>/<procedure>`), never by echoing the query's
  wildcard selector. Zenoh's default reply consolidation keeps one reply
  *per reply key* — distinct origins on distinct keys survive it; a fleet
  replying on the shared wildcard key is consolidated down to one survivor.
  Callers MAY additionally disable consolidation (`None`) for belt and
  braces.
- **Attribution.** The caller joins the reply set against the liveliness
  roster (`<base>/v1/*/state/*/alive`, [04-planes.md §5](04-planes.md)) to
  attribute non-replies — the reply set alone cannot say who *should* have
  answered.
  Several instances of one read procedure — several historians, one per
  host, each answering `range` from its own ring ([11 §2](11-zensight-profile.md))
  — are ordinary fan-in under this rule: each replies on its own concrete
  key and the caller joins them the same way (v1.31).
- **Write fan-out.** A fan-out call — a `*` origin, or any wildcard — to a
  `kind = "write"` procedure whose entry is not `fanout = "allowed"` MUST be
  refused three times over: by the builder (no `FleetSelector` overload is
  generated for it), by the registry (admission rejects the shape), and
  **by the server** (v1.38) — the procedure answers `error/fanout-forbidden`
  on `reply_err` to any query whose key expression is not exactly its own
  concrete key, before its handler runs, with a message naming both keys.
  The first two layers are the *caller's*, and a raw `get` has neither; the
  third is the one every caller passes through. The ACL is **not** a layer
  here, and until v1.38 this bullet said it was: a deny rule fires only when
  it *includes* the query's key expression ([09 §3](09-operations.md)
  fact 6), so under `default_permission: "allow"` a query broader than the
  rule is forwarded to every queryable it intersects, and under `"deny"` it
  crosses whenever an allow rule includes it — which the console's fleet
  grant does. Fan-in is safe for *reads* — collect every host's answer —
  but a broadcast *write* is a fleet-wide side effect, and one mistargeted
  `*` actuates every host at once. The `*` origin stays legal only for
  `read`/`long-running` and for writes explicitly marked
  `fanout = "allowed"` ([08-registry.md §2](08-registry.md)).
  A **dynamic caller** — one holding no generated builder, an explorer
  above all — that cannot establish a procedure's kind (no registry, a
  registry that does not declare it, or a kind token it cannot read) MUST
  treat a fan-out call to it as a write and refuse it unless its operator
  acknowledges the fan-out explicitly (v1.48): not knowing the declaration
  is not a licence, and RFC 08 §2 defaults a write to forbidden. The
  convention's own procedures carry their kind with no registry —
  `introspect`, `describe` and `config/<r>` are reads, every other
  `config/…` key a write (§5.1).

Checklist, because every one of these has been shipped wrong at least once:

| | caller | producer |
|---|---|---|
| target | `All` — **not** the default `BestMatching` | — |
| key | the origin you resolved ([06 §6](06-identity.md)), or an explicit `*` | your **own concrete** key |
| `complete` | — | **never** on an `@rpc` queryable |
| consolidation | `None` for belt and braces | — |
| missing replies | join against the liveliness roster | — |
| a write's target | the concrete key, never a wildcard | refuse any other query: `error/fanout-forbidden` (v1.38) |

> **Editorial note (v1.2).** This section is **correct as written** and was
> deliberately left unchanged by the v1.2 amendments. Both of its MUSTs were
> hit as real bugs during the reference migration — and the cause was not a
> gap in this chapter, it was **not having read it**. The mechanism was then
> "rediscovered" from Zenoh's API docs, which said exactly what §2.1 already
> said. Recorded here so that a future reader who arrives via those bugs does
> not conclude the spec was silent and "fix" a section that was right.
>
> **Editorial note (v1.38).** One thing in it *was* wrong: the write
> fan-out bullet named the ACL as the third refusal, and an ACL cannot
> refuse a query broader than its rule. The bullet now names the server,
> and the reserved vocabulary in §3 gained the name it answers with.

## 3. Read, write, and long-running procedures

Three procedure idioms, distinguished in the registry, all on the same key
shape:

**Read** — idempotent detail queries. High-cardinality data (processes,
sockets, flows, log lines) is held in bounded rings at the producer and
served on demand; it never rides the data classes
([04-planes.md R3](04-planes.md)). Replies are `Vec<Record>` of the
registered reply type — or, for a reply that is bounded and may stop
early, the §3.2 envelope around that list (v1.31).

**Write** — instructions with immediate effect (`set`, `apply`, `trigger`).
The GET carries the instruction as its payload. **A value reply always
means success; a failure always rides Zenoh's reply-error channel**
(`reply_err`) — never a success payload carrying `ok: false` (the D-Bus
guideline verbatim: "a reply always indicates success, and an error always
indicates failure"; [10-prior-art.md](10-prior-art.md)). The error payload
is:

```
{ "error": "<name>", "message": "<human text>" }
```

where `<name>` is machine-readable and namespaced like a key. The
convention reserves `error/invalid-args`, `error/unauthorized`,
`error/not-found`, `error/unsupported`, `error/busy`, `error/gated`, and
(v1.38) `error/fanout-forbidden` — a broadcast reached a write whose entry is
not `fanout = "allowed"`, refused at the server (§2.1);
producer-specific names live under `error/<producer>/…` and are declared
as `[[error]]` entries ([08 §2](08-registry.md), v1.40) — linted, pinned
in the lock, served by `introspect`, and retired through `[[deprecated]]`
with `kind = "error"`, so deprecate-never-reuse applies to them exactly as
it does to a subject. (Through v1.39 this sentence said they were
"registered like subjects", and no entry kind existed to register them.) A successful write replies with an
empty or result-bearing value. (Envelopes are shown as JSON for
readability; the wire encoding is the deployment's payload default,
CBOR in the reference application.)

A write procedure MUST reply — so for a write, *silence is never a
refusal* (refusals are error replies; see §3.1 for what silence does
mean). Idempotency is per-procedure and MUST be documented in the registry
entry. Gated/dangerous writes (service actions) keep their gate at the
server (allowlist/polkit) and refuse with `error/gated`; the convention
adds ACL-by-prefix as an outer layer, since `…/h-xxx/@rpc/systemd/action`
is a literal key an ACL can deny per client.

**Long-running** — anything that outlives a query timeout (artifact
generation, captures). The pattern is *RPC to initiate, state to observe*:

1. `GET …/@rpc/<producer>/artifact/request` (body: kind + options) →
   `{ id }` immediately (or an error reply);
2. progress is ordinary observable state:
   `…/state/<producer>/artifact/<kind>` (LWW status document —
   generating/ready/failed/expired, tombstoned when freed);
3. completion may additionally emit an `events` record
   (`…/events/<producer>/artifact/<ulid>`) for the audit trail;
4. the bytes are pulled from `@blob` ([07-bulk-planes.md](07-bulk-planes.md));
5. `GET …/@rpc/<producer>/artifact/cancel?id=<ulid>` frees early.

This replaces the incumbent trio of a pub/sub request key, a status
queryable, and a cancel subscriber with one uniform mechanism — and makes
progress visible to *every* observer (it is state), not only the requester.

**No durable commands.** RPC is synchronous-ish: an offline host misses the
call, and the caller can *determine* that it did (§3.1). If a deployment
ever needs "instruction that survives producer downtime", the escape hatch
is desired-state reconciliation — the controller publishes
`state/<producer>/desired/<topic>` and the producer converges on
(re)connect. When the controller does **not** run on the target host, it
authors that desired-state under a **registered service origin** with the
target as the first subject chunk
(`<base>/v1/@desired/state/h-xxx/config/<if>/desired`) — the
service-origin carve-out in [07-bulk-planes.md §3](07-bulk-planes.md),
grammar-legal with zero new mechanism. No current channel needs it; decided
(RPC-only, escape hatch sanctioned) in
[12-open-questions.md §3](12-open-questions.md).

### 3.1 What silence means (normative honesty)

"No reply" is not one condition. With plain GET semantics it conflates:

| Cause | Observable behavior |
|---|---|
| no queryable matches (offline host whose session expired, mistyped origin, procedure not served) | the query finalizes **empty, fast** |
| host reachable but session dying / mid-boot before queryable declaration | empty at the **query timeout** (default 10 s) |

Callers therefore MUST NOT treat "empty reply set" as a verdict about any
specific host. The discipline that makes silence attributable:

- producers declare `@rpc` queryables **before** their `alive` liveliness
  token ([04-planes.md §5](04-planes.md)), so *alive ⇒ callable* — a host
  that is alive on the roster but silent on RPC is a bug, not a boot race;
- callers consult the liveliness roster to classify: not on roster =
  offline/unenrolled; on roster + error reply = refused; on roster + no
  reply within timeout = investigate.

### 3.2 Bounded and computed answers (normative, v1.31)

§4 reserves RPC for what state cannot express — parameterised,
high-cardinality, or *computed* replies — and every such reply is bounded:
a ring has a size, a scan has a cap, a downsampled series has a tier that
covers some window and not another. A bounded reply has to be able to say
two things a bare list cannot: **"there is more"** and **"I stopped
early"**. Without the second, a search that hit its scan cap with zero
matches, a range whose tier could not cover the window asked for, and a
filter applied after the page cap all return a well-formed short page —
and the caller concludes end-of-history. Four handlers of the reference
application did exactly that before this section existed.

A reply that is paginated or that MAY stop early is therefore an
**envelope**, not a list:

```json
{
  "items":       [ ... ],
  "next_cursor": "<opaque>" | null,
  "partial":     false,
  "scanned":     4096,
  "covers_from": "<instant>" | null
}
```

- `items` is the page, in the order the procedure documents.
- `next_cursor` non-null means more; `null` means the walk is complete
  **for the filter given**. A cursor is opaque to the caller but MUST be a
  **value** cursor — the last emitted key, id or instant — never a
  position: a set that grows or shrinks between pages otherwise skips or
  repeats silently, and the reference historian had exactly that defect.
- `partial: true` means the producer stopped before completing the walk —
  scan cap, tier coverage, time budget — and the caller MUST NOT treat a
  short page as the end. It MAY continue from `next_cursor` when one is
  offered. `partial: true` **with** `next_cursor: null` is a contract
  violation: the producer says it stopped early and offers no way on. An
  observer MAY report it as a finding ([13 §3](13-observer-conformance.md)).
- `scanned` is advisory — what the page cost — so a caller can tell an
  expensive empty page from a cheap one. It MAY be omitted.
- `covers_from` states the oldest instant the answer *could* have covered,
  for a computed answer whose coverage is narrower than what was asked: a
  sub-minute query over a hot ring that answers ten minutes as if they were
  the whole day is the "stopped early" gap in time rather than in count.
  It MAY be omitted by procedures that have no notion of coverage.

The envelope is a reply *type* like any other: the registry declares the
procedure's `reply` as the application's page type (`Page<T>` in its type
table, [08 §5](08-registry.md)), and a procedure that shipped a bare list
migrates by [08 §3](08-registry.md)'s rule — a new procedure with the
envelope reply beside the old one, the old one deprecated — because a
reply shape is a contract and a list that becomes an object is a break.
What is deliberately *not* here: a query language, a total count (a bounded
producer cannot know it without the unbounded walk this section exists to
avoid), and a server-side session — the cursor is the whole state.

## 4. Late-joiner seeds are state, not RPC

The incumbent keyspace used queryables to seed late joiners (firing alerts,
stream catalogues, entity sets). Under this convention those seeds are
unnecessary as separate endpoints: the data *is* `state`, and a late joiner
seeds from the same keys it will then watch — a dedicated `query/alerts`
procedure would merely duplicate the state selector. RPC is reserved for
what state cannot express: parameterised, high-cardinality, or computed
replies. *How* to seed correctly (subscribe-first, timestamp merge, the
two seed paths and their composition) is the delivery contract's seed
discipline, defined once in [04-planes.md §3.2](04-planes.md).

## 5. Mapping incumbent channels (the pattern)

An application migrating onto this convention re-homes each of its
existing control channels by *mechanism*, not by name — the row-by-row
table for the reference application's channels is profile material and
lives in [11-zensight-profile.md §5](11-zensight-profile.md) (moved there
in v1.25). The neutral pattern, which any adopter's table instantiates:

- **A command/status/query triple becomes its three native planes.** A
  config-style topic `<topic>` maps to `@rpc/<producer>/<topic>/set`
  (write, ack reply) + `@rpc/<producer>/<topic>` (read current); an
  observable status is a `state` document, not a queryable.
- **Seed queryables dissolve into state selectors** (§4): a "current
  alerts" endpoint becomes a GET on the `state/*/alert/*` selector.
- **Long-running work is the §3 idiom**: request/cancel are `@rpc`
  writes, progress is `state/<producer>/<job>/<kind>`, completion may
  emit an `events` record, bytes ride `@blob`
  ([07-bulk-planes.md](07-bulk-planes.md)).
- **A stream catalogue is per-stream state.** Streams and their status
  are LWW documents at `state/<producer>/stream/<stream>` — one document
  per stream, control via `@rpc` writes ([§3](#3-read-write-and-long-running-procedures)),
  the offered tiers advertised in the document
  ([07-bulk-planes.md §1](07-bulk-planes.md)). A closed stream keeps its
  document (with `open: false`); the tombstone marks *removal from
  config*, not closing, or consumers lose the "openable streams"
  catalogue.

Two systematic effects of the mapping:

- **Every host-vs-protocol scoping asymmetry disappears.** sysinfo's
  host-scoped queries and netlink's protocol-scoped ones become the same
  shape; "which scope does this channel use?" is no longer a question the
  consumer can get wrong.
- **Status keys stop being a third mechanism.** What was
  command/status/query triples becomes: writes (RPC), reads (RPC), and
  observable state — each in its native plane.

### 5.1 Configuration (normative, v1.42)

The first bullet of §5 is the skeleton every adopter starts from — a
`set`, a read, a `state` echo — and every adopter has then invented the
rest for itself, because the thing being configured is often the thing
the call travels through. This section is the rest, once. It is normative
for a producer that declares a **configuration resource**; a producer
that has nothing to configure declares none and owes nothing here.

**Keys.** A resource `<r>` — a device, an interface, a namespace: the
chunk an ACL grants by ([09 §3](09-operations.md)) — has:

| Key | Kind | Carries |
|---|---|---|
| `@rpc/<producer>/config/<r>` | read | the read-back document (below) |
| `@rpc/<producer>/config/<r>/<group>/set` | write, `fanout = "forbidden"` | a change to one group |
| `@rpc/<producer>/config/<r>/confirm`, `/cancel`, `/extend` | write | `{token}` — the pending change's; `extend` adds `confirm_s`, the new window from now |
| `@rpc/<producer>/config/<r>/persist` | write | `{token}` — the pending or last confirmed change's; **its own key**, so an ACL can allow a change and deny making it survive a restart |
| `state/<producer>/config/<r>` | state, `transition` or stronger ([04 §3](04-planes.md)) | the read-back document, refreshed on every change |
| `events/<producer>/config_change/<ulid>` | events, `low` | the change event (below) |

**The schema is served, not documented.** The read-back document
carries, beside every value, the declaration it satisfies: the resource's
**groups**, each with a **class** and its **parameters** — name, kind
(`bool` | `integer` with optional `min`/`max`/`unit` | `text`), a
one-line description, and whether it is **sensitive**. A tool renders a
form from it without being compiled against the producer, and `describe`
([08 §7](08-registry.md)) covers the document's own shape. The reference
types are `zenkey::config`.

**Groups are the resource of a write.** A group is the set of parameters
that change together — a frequency with its bandwidth, a spreading
factor with its coding rate — because a single-parameter intermediate
state of a coupled pair can be unsafe, and because a group is the unit
that has a class and that an ACL sees in the key. A `set` addresses one
group; a subset of its parameters is a change of those alone.

**Three classes, and the class belongs to the producer's declaration.**

- **hot** — takes effect at once and nothing above the producer needs to
  know: a transmit power, a poll interval, a queue length. The reply to a
  `set` is the read-back document, never an echo of the request.
- **reach** — decides whether the producer can reach the bus at all: a
  frequency, a network id, an APN. A `set` on a reach group MUST carry a
  confirm window (`confirm_s`) and is answered `{token, apply_at}`
  **before** it is applied, because the read-back would cross the link
  being changed: the caller observes `state/<producer>/config/<r>` over the
  new link, then confirms. This is §3's long-running idiom, and the reach
  class is why it exists here. A reach group MUST NOT be carried by
  desired state ([12 §3](12-open-questions.md)): one bad desired publish
  would lock the node out of its own supervision. A tool that cannot
  read a group's class — no read-back to read it from — SHOULD treat a
  change carrying `confirm_s` as reach when it asks its operator's consent
  (v1.48): a window is how a reach change is sent, and asking once too
  often costs a keystroke where not asking can cost the link.
- **contract** — part of what the producer's transport was started
  against: an SDU size, a reliability claim. Refused at runtime with the
  restart named (`error/<producer>/restart-required`, an `[[error]]` entry);
  changed in the producer's own startup configuration.

The same parameter can be `hot` on one producer and `contract` on
another — an MTU is hot where the transport sizes itself from the link and
contract where the transport was started against it — so the class is
declared per group, never inferred from a name.

**A sensitive parameter is write-only.** Declared `sensitive`, it is never
read back, published, logged or carried in an event: its read-back entry
has no value, and its edit in a change event is `redacted`. This is
[RFC 8341](https://www.rfc-editor.org/rfc/rfc8341.html)'s
`default-deny-all` as data, and the companion marker for ACL generation.

**A change request** (`set`'s body) carries the group's values and, each
optional: `expected_revision`, refused if the document has since moved;
an `idempotency_key`, so a retried request returns the first answer
instead of applying twice — a lost reply is otherwise a doubled write;
`dry_run`, which validates and reports what would change without
touching the device; and `confirm_s`, which arms a rollback.

**Validation is the producer's, before its device sees anything**, and
the reference validator (`zenkey::config::ConfigSchema::validate`) is
what makes every producer refuse the same input in the same words: an
unknown group or parameter is `error/not-found`; a wrong kind, an
out-of-bounds value, a reach change without a window or a stale
`expected_revision` is `error/invalid-args`; a contract group is the
producer's `restart-required`; the device's own refusal is the producer's
`device-refused`, with the device's words as the message.

**Confirmed commit.** A change with `confirm_s` is applied and a
deadline armed. `confirm` makes it permanent, `cancel` undoes it now,
`extend` moves the deadline; each carries the change's token. At the
deadline the producer applies the undo **once** — never retried, never
escalated: an undo that fails ends at outcome `partial`, and the
read-back is the truth. This is
[RFC 6241 §8.4](https://www.rfc-editor.org/rfc/rfc6241.html), MikroTik's
Safe Mode and airOS's test mode, and it is the whole reason a reach change
is survivable. A restart during a pending change is a rollback by
construction, because a runtime change is not persisted.
`confirm`, `cancel`, `extend` and `persist` each answer with the
read-back document as it stands after the act (v1.47), so the caller sees
the outcome without a second call; a token that names no pending change
— for `persist`, no pending or last confirmed change — is
`error/not-found`.

**One pending change per resource.** A second writer is answered
`error/busy` naming the pending token, unless it carries that token.
This is [03 §1.5](03-grammar.md)'s single-writer rule, for configuration.

**Persistence is a separate, deniable act.** `persist` writes a confirmed
change into the producer's own persisted layer — where and how is the
producer's; that it is explicit and separately authorised is this
section's. The read-back names each value's **source** (`default`,
`file`, `overlay`, `runtime`) and its startup value where the two differ,
so divergence is visible without a second document.

**Every change is an event**, shaped after
[RFC 6470](https://www.rfc-editor.org/rfc/rfc6470.html)'s
`netconf-config-change`: the resource, the new `revision`, the `token`
if any, an `outcome` (`applied` | `confirmed` | `rolled-back` |
`partial`), the `edits` as `{parameter, old, new}` with sensitive values
redacted, and the attribution — `actor` and `request_id` as the caller
spelled them in the selector, and `claimed_source` as the transport
reported it. **None of that authenticates.** [06 §5.5](06-identity.md)
names an *actor* without defining it; this is the definition: a
caller-claimed label, carried for the record. The ACL is the authority,
and the reference tooling spells the two parameters `?actor=` and
`?request_id=`.

**The server's own guards stay at the server** (§3): a write refuses a
query whose key expression is not its own concrete key (§2.1,
`error/fanout-forbidden`), and a producer MAY keep a startup allowlist of
writable groups, answering `error/gated` for the rest — so an operator
switches remote writes off per group without touching a router.

**Deliberately not here.** A datastore model beyond running plus an
explicit persisted layer; a coordinated change across producers at an
agreed time, which needs a clock both trust; and any schema kind beyond
the three — a fourth arrives with the first producer that needs it.
