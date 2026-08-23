# 05 — Control Plane: `@rpc`

**Status: v1.2 (ratified)** · normative chapter · *amended in v1.2 and v1.25 — see [CHANGELOG.md](CHANGELOG.md)*

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
- **Write fan-out.** A fan-out (`*`-origin) call to a `kind = "write"` /
  `fanout = "forbidden"` procedure MUST be refused — by the builder (no
  `FleetSelector` overload is generated for it), by the registry (admission
  rejects the shape), or by the ACL, in that order of preference. Fan-in is
  safe for *reads* — collect every host's answer — but a broadcast *write*
  is a fleet-wide side effect, and one mistargeted `*` actuates every host
  at once. The `*` origin stays legal only for `read`/`long-running` and for
  writes explicitly marked `fanout = "allowed"`
  ([08-registry.md §2](08-registry.md)).

Checklist, because every one of these has been shipped wrong at least once:

| | caller | producer |
|---|---|---|
| target | `All` — **not** the default `BestMatching` | — |
| key | the origin you resolved ([06 §6](06-identity.md)), or an explicit `*` | your **own concrete** key |
| `complete` | — | **never** on an `@rpc` queryable |
| consolidation | `None` for belt and braces | — |
| missing replies | join against the liveliness roster | — |

> **Editorial note (v1.2).** This section is **correct as written** and was
> deliberately left unchanged by the v1.2 amendments. Both of its MUSTs were
> hit as real bugs during the reference migration — and the cause was not a
> gap in this chapter, it was **not having read it**. The mechanism was then
> "rediscovered" from Zenoh's API docs, which said exactly what §2.1 already
> said. Recorded here so that a future reader who arrives via those bugs does
> not conclude the spec was silent and "fix" a section that was right.

## 3. Read, write, and long-running procedures

Three procedure idioms, distinguished in the registry, all on the same key
shape:

**Read** — idempotent detail queries. High-cardinality data (processes,
sockets, flows, log lines) is held in bounded rings at the producer and
served on demand; it never rides the data classes
([04-planes.md R3](04-planes.md)). Replies are `Vec<Record>` of the
registered reply type.

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
`error/not-found`, `error/unsupported`, `error/busy`, `error/gated`;
producer-specific names live under `error/<producer>/…` and are registered
like subjects — deprecate-never-reuse applies
([08-registry.md](08-registry.md)). A successful write replies with an
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
