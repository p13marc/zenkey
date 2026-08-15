# 07 — Bulk Planes: `@media` and `@blob`

**Status: v1.7 (proposed)** · normative chapter · *amended in v1.2, v1.7, v1.8, v1.11 and v1.16 — see [00-index.md](00-index.md)*

Two kinds of traffic must never meet a wildcard: frame-rate opaque bytes
(video, imagery) and bulk transfers (files, directory trees, chunks). Both
get verbatim planes in the class position, so no data selector — not even a
per-origin firehose `…/h-xxx/**` — can ever pull them by accident
(design property D2, [03-grammar.md §4](03-grammar.md)).

---

## 1. `@media` — live opaque streams

```
<base>/v1/<origin>/@media/<producer>/<stream>/video/<codec>/<tier>
<base>/v1/<origin>/@media/<producer>/<stream>/preview/<format>
```

Example: `zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/high`,
`…/@media/parallax/cam0/preview/jpeg`.

The last video chunk is a **tier** — a named bandwidth rung (`low` /
`medium` / `high`) the publisher offers concurrently, one encoder each, on
distinct keys. It is *not* an H.264 profile: it names the rung, not the
bitstream's coding profile. A producer publishes several tiers of one stream
at once (demand-driven — a tier costs nothing until it has a subscriber), and
each viewer subscribes to the single tier its link can take. This is what
lets two operators on different links watch the same camera without fighting
over one encoder's settings (the constrained-link viewer picks `low`, the LAN
viewer keeps `high`, and neither move touches the other). The offered tiers
are advertised in the stream catalogue (the `streams` procedure's
`StreamDescriptor`), so a viewer *can* know them.

Rules:

- **Payload is raw encoded bytes** (access units, JPEG frames) with the
  container/codec declared via the middleware `Encoding`
  (`video/h264`, `image/jpeg`) — never a telemetry envelope, never fed to a
  telemetry decoder.
- **Frame metadata rides the attachment**, as a compact binary document
  (reference: CBOR `FrameMeta` — keyframe flag, pts/dts/duration,
  sequence, dimensions). Keys stay stable; per-frame data never touches
  them ([03-grammar.md §2](03-grammar.md)).
- **QoS: best-effort · drop · interactive-high** — a stale frame is
  worthless and the encoder must never block ([04-planes.md §3](04-planes.md)).
  Plain declared publisher; explicitly **never** an AdvancedPublisher —
  no cache, no miss detection, no heartbeat (recovering a superseded frame
  is anti-useful; [04-planes.md §3.3](04-planes.md)).
- **Keyframe-on-subscribe**: the publisher SHOULD watch subscriber matching
  (matching listener) and force a keyframe when a viewer arrives, and the
  keyframe flag MUST be a byte-level promise — a fresh decoder can start at
  any sample whose attachment says keyframe (parameter sets inline or
  prepended). Note the matching listener signals only the
  no-viewers ↔ some-viewers *edge*: an Nth viewer joining beside a current
  one produces no event and obtains its immediate keyframe via
  `@rpc/<producer>/stream/keyframe`
  ([05-control-rpc.md §3](05-control-rpc.md)) instead of waiting out a GOP.
- **Viewer selectors are exact, on both media shapes.** A preview subscribes
  to its exact `…/preview/<format>` key; a video viewer subscribes to the
  exact `…/<stream>/video/<codec>/<tier>` key of the one tier it chose. There
  is **no wildcard on `@media`**. The old `…/video/<codec>/*` license (v1.1)
  rested on "the viewer cannot know the last chunk" — but the catalogue now
  publishes the tier list, so the viewer *can* know it, and §3's rule
  ("wildcard only a chunk you cannot know") forbids the `*`. The license is
  **revoked**, and this is load-bearing: with tiers published concurrently, a
  `…/video/h264/*` subscriber would match *every* tier at once and receive
  several interleaved H.264 streams on one subscriber, unseparable except by
  re-parsing the key per sample. Exact-tier subscription is the whole point —
  the subscription *is* the quality choice.
  In particular a viewer MUST NOT wildcard the **origin**: `…/*/@media/…`
  subscribes to *every host in the fleet* publishing a stream of that name
  and decodes all of them to render one tile — the same amplification §2
  forbids as a default `@blob` fetch path, on the plane that carries the
  most bytes per second on the bus. A viewer that does not know which host
  it is looking at has not finished resolving its target
  ([06-identity.md §6](06-identity.md)), and MUST resolve it rather than
  paper over it with a wildcard.
- **Stream control is `@rpc`**, stream status/catalogue is `state`
  ([05-control-rpc.md §3](05-control-rpc.md)); stream *stats*
  (fps/kbps/drops/viewers) are ordinary `telemetry` under
  `telemetry/<producer>/<stream>/stats/…` — charts light up for free.
  `@media` carries pixels and nothing else.

## 2. `@blob` — bulk and content-addressed transfer

*Re-specified in v1.7 against the reference client's verified-streaming
protocol. The plane's placement, position count and Tier-1 shape are
unchanged; what changed is that Tier-2 keys are now genuinely
content-addressed, the per-artifact endpoints are named, and integrity is
anchored rather than assumed.*

```
<base>/v1/<origin>/@blob/artifact/<id>/**             Tier-1: one named blob, verified streaming
<base>/v1/<origin>/@blob/tree/<root>                  Tier-2: directory-tree index, keyed by its own root hash
<base>/v1/<origin>/@blob/store/<algo>/<hash>          Tier-2: content-addressed chunk (immutable)
```

The chunk after `@blob` is a reserved **tier token** (`artifact` | `tree` |
`store`), not a producer chunk ([03-grammar.md §1.5](03-grammar.md)) —
content-addressed data has no owning component. The tiers are **pull-only**
(a consumer that never asks never pays a byte), fronted by a resumable
verifying client (reference: [`zblob`](https://github.com/p13marc/zblob)).

### 2.1 Integrity is anchored, not assumed (normative)

> **Every `@blob` reference handed to a consumer MUST carry the identity of
> the bytes it names.** Tier-1: the reference MUST include the blob's
> content root. Tier-2: the key *is* the root (§2.3), so this holds by
> construction.

A blob reference travels over the same bus as the blob. Without an anchor
the consumer can only trust whoever answers first — integrity then holds
*within* a transfer (a server cannot mix content) but not *across* it (the
server still chooses which content). That is trust-on-first-use, and it is
not what a fleet-wide content plane should offer by default.

The anchor makes verification total: the reference client verifies each
reply against the anchor **before it reaches disk**, so a wrong or tampered
reply is discarded rather than assembled and detected later. This supersedes
the v1.2 description ("hash verification" of a whole blob after transfer),
which could only fail a transfer *after* paying for all of it.

Practically: the RPC that mints an artifact ([05-control-rpc.md §3](05-control-rpc.md))
MUST convey the root alongside the id and prefix, and durable state that
advertises a tree (§2.3) advertises its root because that *is* the key.

### 2.2 Tier-1 — `artifact/<id>`

Whole-file delivery of a one-off artifact (debug bundle, pcap). The `<id>`
is the ULID minted by the RPC that created it; the id is per-artifact, which
is acceptable *here* because blob keys are short-lived queryable endpoints,
not published state.

> **`<id>` is one plain chunk, and a ULID enters it lowercased**
> ([03 §2](03-grammar.md)). A canonical ULID is uppercase Crockford base32
> and uppercase has no spelling in a non-verbatim chunk, so an id copied
> from an RPC reply verbatim produces a key this convention cannot parse:
> `…/@blob/artifact/01HQXK8F9C2N4P/manifest` is not a v1 key, and an
> explorer that attributes a reply *by its key* ([05 §2.1](05-control-rpc.md))
> then cannot name which origin answered. Crockford base32 decodes
> case-insensitively, so the lowercasing is lossless and the payload MAY
> carry the canonical uppercase form.

The endpoints under `artifact/<id>/` are reserved and normative. The table
separates the key a consumer **asks on** from the key a reply **arrives
on**, because the two are not always the same key and the difference is
load-bearing (below):

| Request key | Reply key(s) | Kind | Purpose |
|---|---|---|---|
| `<id>/manifest` | same | queryable | sizing + chunking + the content root |
| `<id>/**` + chunk-range selector | `<id>/slice/<i>` | queryable | verified slices of the requested transfer chunks |
| `<id>/have` | same | queryable | availability: which chunks this origin can serve |
| `<id>/push/offer` | same | queryable (write) | upload offer (see below) |
| `<id>/push/slice/<i>` | same | queryable (write) | one uploaded slice |

*(A sixth endpoint, `<id>/fanout`, was normative here from v1.7 to v1.16.
It is demoted to [Appendix A](#appendix-a--fanout-experimental) in v1.17 —
kept on record, experimental, no longer promised by this table.)*

**Why the second column exists (normative).** Zenoh delivers a reply only
if its key intersects the query's key expression, and the check runs on the
*serving* side — a mismatched reply fails at the holder, and the consumer
observes silence rather than an error. An endpoint whose replies are
per-chunk therefore cannot be asked for on the reply key: a slice is
requested as a **chunk-range selector on `<id>/**`**, and each slice
arrives on its own `<id>/slice/<i>`. That makes `slice/<i>` a key a
consumer *receives*, never one it GETs — an implementer reading a
one-column table as "GET `<id>/slice/<i>`" builds a client that asks on
keys no holder serves and receives nothing, the exact silent failure mode
this convention works hardest to design out elsewhere. The same
request/reply split recurs on Tier 2's batched fetch (§2.4), which is
additionally the one sanctioned exception to the intersection rule itself.

One correction to v1.2 stands unchanged: `push/**` is a **write path
expressed as a query** (the payload rides the GET): the uploader offers a
manifest, the origin answers with the chunk ranges it still wants, and each
pushed slice is verified against the offered root before it is retained.
This is not an exemption from the declared-publisher rule
([04-planes.md §3](04-planes.md)) because nothing is published — but it
*is* a write, so it MUST be gated by an authorization hook on the receiving
origin and MUST NOT be enabled by default.

**`push` remains normative**, and v1.17 says so explicitly so that the
demotion of `fanout` beside it is not read as applying to it. The two are
not in the same position: `push` is the plane's *only* write path, its
authorization gate is a designed-in property rather than an afterthought,
and a foreseeable first consumer exists (a support-bundle upload). An
endpoint being unconsumed today is an argument about tables describing
what is served — it is not, by itself, an argument for removing the one
write affordance the plane has.

Resume is a persisted **chunk bitfield**: the client re-requests exactly the
holes it is missing, as a chunk-range selector on the same wildcard GET. A
missing chunk in the middle no longer re-streams everything after it.

### 2.3 Tier-2 — `tree/<root>` + `store/<algo>/<hash>` (changed in v1.7)

> **A tree index is keyed by its own root hash.** `<root>` is the index's
> content root in the same hex spelling as `<hash>`. Snapshot *names* are
> not `@blob` keys — a name is durable mutable state and belongs on `state`.

v1.2 asserted that "tree ids are root hashes" and rested three conclusions
on it: fleet-wide cacheability, the sanctioned PUT exemption, and
last-writer-wins reconciliation being a no-op. The assertion was never
enforced, and the reference implementation allowed a caller-chosen name
(`tree/nightly`), which makes all three false at once: the key is mutable,
so a cached copy can be stale, two producers publishing under one name
silently clobber, and the "immutable ⇒ cacheable" argument evaporates
exactly where the storage exemption needs it. v1.7 makes the assertion true
by making it the rule.

The consequence is a clean split, and it is the familiar one (git objects
and refs; see [10-prior-art.md](10-prior-art.md)):

- **Immutable content lives on `@blob`.** `tree/<root>` and
  `store/<algo>/<hash>` are pure content addresses. Re-publishing is a
  byte-identical no-op. Any holder's copy is as good as any other's.
- **Mutable names live on `state`.** "Which snapshot is current" is a
  durable fact about a producer, so it is ordinary published state —
  `state/<producer>/snapshot/<name>` carrying the root — with the class
  semantics, storage behaviour, and late-joiner seeding every other durable
  fact already gets. It costs one small sample, and it is versioned,
  cacheable and observable like the rest of the plane.

A consumer therefore resolves a snapshot in two hops: read the name from
`state` to obtain a root, then fetch `tree/<root>`. The second hop is
self-anchoring (§2.1) — the key it asked for is the identity it must get.

The transfer itself is unchanged in spirit: the client GETs the index, diffs
the hashes it needs against its local content store, fetches only the
missing chunks (re-hashing each on receipt), reconstructs, and verifies the
root. Resume *is* "which hashes I already have" — it survives reconnect and
restart with no session state.

### 2.4 Chunk values are framed; the hash addresses the content (normative)

`<hash>` is the hash of the chunk's **content**, not of the bytes on the
wire. A chunk value is a self-describing container so that transport and
at-rest concerns can vary without changing an address:

- the container declares its own form (uncompressed, or a named compression);
- a receiver unframes first, then verifies the content hash against the key;
- an incompressible chunk is carried uncompressed rather than inflated.

Two rules follow. A holder MAY re-frame a chunk it stores (e.g. compress it)
without changing its key, because the address is unaffected — this is what
keeps fleet-wide dedup intact across holders with different storage
policies. And an **encrypted** container MUST NOT appear under a
fleet-reachable `store` key: encryption at rest is a property of one
holder's disk, not of the shared address space; publishing one would give
every other holder an object it can neither verify nor use.

`<algo>` names the hash function. A deployment SHOULD use one algorithm
fleet-wide (dedup is per-algorithm: the same bytes under two algorithms are
two objects); the segment exists so a migration can run both side by side.

**Reserved Tier-2 endpoint tokens (added in v1.17).** Tier 1 reserves its
endpoints by name (§2.2); until v1.17 Tier 2 reserved none — the key *was*
the endpoint, and [08 §2](08-registry.md) leaned on that sentence. Wire v3
introduces two tokens, and the moment Tier 2 has any endpoint at all, what
kept the keyspace unambiguous must be stated as a rule rather than left as
an accident:

| Request key | Reply key(s) | Purpose |
|---|---|---|
| `store/<algo>/batch` | `store/<algo>/<hash>`, one per delivered chunk | batched chunk fetch: the request carries a want-list of content addresses |
| `store/<algo>/have` | same | Tier-2 store probe (§2.5) |
| `tree/<root>/have` | same | Tier-2 tree probe (§2.5) |

- **A reserved Tier-2 token MUST NOT be a valid content address under any
  registered `<algo>`.** Today that holds by accident — content addresses
  are hex, `batch` and `have` are not — and this rule is what keeps it
  holding when the next algorithm or the next token arrives.
- **Tier-2 keys resolve positionally.** The chunk after the tier token is
  `<algo>` (or `<root>`); the chunk after *that* is either a content
  address or a reserved token, distinguished by the rule above. An
  implementation MUST NOT resolve these keys by string-prefix matching
  against configured prefixes — Tier 1 already resolves `<id>`
  positionally, and Tier 2 now does the same.
- **`batch` replies arrive on each chunk's ordinary store key, and that is
  a requirement, not an implementation detail.** It keeps every delivered
  chunk individually verifiable against its own address and individually
  cacheable, and it lets a router storage (§2.5) serve as singles what it
  cached from a batch. Because those reply keys do not intersect the
  request key, `batch` is the **one sanctioned exception** to §2.2's
  reply-key-intersection rule: a `batch` GET MUST declare that it accepts
  non-matching replies, and a holder MUST NOT batch-reply to a query that
  did not so declare.
- **A router storage never answers `batch`.** It serves by key, and no
  `…/batch` key exists in it, so it stays silent — which is safe, and has
  a consequence a client MUST implement: fall back to per-chunk GETs for
  every hash a batch round leaves unanswered. The fallback is not a
  nicety; it is what keeps the router-store tier (§2.5) usable by a
  batching client at all.

### 2.5 Fleet-wide caching and the router store

**Chunks and trees are immutable ⇒ cacheable fleet-wide.** Replies are valid
from *any* holder. The normative dedup point is a **router-hosted content
store**: chunks and indexes MAY be PUT into router storages on the
`…/@blob/store/**` and `…/@blob/tree/**` selectors (the sanctioned exemption
from the declared-publisher rule, [04-planes.md §3](04-planes.md)) so a
producer publishes once and exits, and the fleet fetches the router copy.
§2.3 is what earns this exemption: both families are now content-addressed,
so a storage's last-writer-wins reconciliation is genuinely a no-op.

A publisher MUST NOT treat a resolved PUT as durability: it signals hand-off
to the transport, not retention by a storage, and index and chunks may land
on different storages with no ordering between them. A producer that intends
to exit MUST confirm retention by reading back what it published (the index
and a sample of chunks) before considering the snapshot available.

**Probing is a named endpoint, not a wildcard convention.** A consumer that
cannot name the origin holding a blob probes with a *tiny* reply — `have`
(availability) or `manifest` (§2.2) — across origins, then fetches from one
chosen origin's concrete key. This supersedes v1.2's advice to use "manifest
/existence checks with tiny replies": the probe now has a purpose-built
endpoint whose reply is a bitfield, and a client can use it to choose the
best-stocked holder rather than merely the first to answer. The prohibition
itself is unchanged and is stated once, normatively, in §3: a wildcard-origin
*bulk fetch* remains forbidden as a default path, because every matching
holder ships the full payload and Zenoh cannot cancel remote replies in
flight — N holders cost N× the bytes, amplification on exactly the links
this plane promises to spare.

### 2.6 QoS: bulk yields — a client obligation

Zenoh replies inherit the *query's* QoS (server-side reply-QoS setters are
no-ops), so it is the `@blob` caller that MUST issue its GETs at data-low
priority; that is what keeps a transfer from starving telemetry or an alert
on a constrained link ([04-planes.md §3](04-planes.md)). The obligation is
unchanged from v1.2 and it is easy to forget, so the reference client now
discharges it **by default** rather than documenting it: a client that never
touches the setting is already conformant, and raising the priority is the
deliberate act.

### 2.7 Registry modelling (added in v1.8)

v1.7 left `@blob` the one plane with no registry entry kind, which meant the
only plane carrying whole files was also the only one no build-lint and no
bus explorer could see. [08 §2](08-registry.md) closes that with a `[[blob]]`
entry kind: an origin declares which **tier** it serves and, for `artifact`,
which of §2.2's endpoints — nothing more, because the key shapes above are
fixed by this chapter and their variable chunks are content addresses rather
than registry vocabulary.

Two rules of this section become *structural* there rather than advisory.
Generated `tree`/`store` builders take a validated content-hash type, so
§2.3's revoked caller-chosen name has no spelling in the generated surface;
and the §2.5 probe form returns a distinct probe-prefix type, so a probe
prefix cannot be passed where the §3 prohibition forbids one. Declaring
`push` in an entry remains a statement of capability — the authorization gate
and the off-by-default posture of §2.2 are unaffected by anything a registry
says.

*Since v1.16 the symmetry this section created is restored the other way:
`[[media]]` declarations (§1's plane) reach the runtime introspect slice
exactly as blob entries do ([08 §2/§6](08-registry.md)), so a viewer can
enumerate an origin's streams off the bus before subscribing to exactly
one — §1's no-wildcard rule needs the enumeration to come from somewhere,
and now it is served rather than compiled in.*

## 3. The wildcard rule (normative)

*Added in v1.2. The `@blob` fan-out caveat in §2 and the `@media` origin
rule in §1 are two instances of one rule that was never stated.*

> **A publisher MUST always use its concrete origin. A subscriber MAY
> wildcard a chunk it cannot know — and only such a chunk.**

The two halves are not symmetric, and the asymmetry is the point.

- **Publishing** is an assertion about *who you are*. There is exactly one
  right answer and the publisher always has it. A `*` in a published key is
  never a shortcut; it is a lie about identity, and it is unrepresentable
  if the origin is a value the publisher owns rather than a string it
  formats ([08-registry.md §1.1](08-registry.md)).
- **Subscribing** is a question about *what exists*. A `*` is the honest
  spelling of "I cannot know this chunk" — the set of producers on a host,
  the hosts in a fleet. (Media tiers were once cited here; they no longer
  qualify — the catalogue publishes them, so a viewer subscribes to an exact
  `<tier>`. §1.)

The test for a subscriber is therefore **"can I know this chunk?"**, not
"is this convenient?". A chunk you *could* resolve but did not is a
wildcard that will one day match more than you meant — and on the bulk
planes, "more than you meant" is measured in megabits.

**Cost is the second gate, and it binds even when the first passes.** A
consumer legitimately unable to name an origin still MUST NOT fan out
across origins on `@media` or `@blob`, because every matching holder ships
the full payload and Zenoh cannot cancel remote replies in flight (§2).
Wildcard-origin on a bulk plane is for *probing* — tiny replies — followed
by a fetch from one chosen origin's literal key. On the data classes
(`telemetry` / `state` / `events`) a wildcard origin is ordinary and
expected; it is what a fleet view *is*.

**Carve-out — a registered service origin publishing on behalf of a target
(normative).** The publisher-side rule assumes the publishing identity and
the *subject* of the data are the same host. One case breaks that
assumption honestly: a controller publishing **durable desired-state** a
target must converge on. Such a publisher is not lying about identity — it
is asserting *its own* service identity as the author of an instruction —
so it does not need a `*`, and MUST NOT use one.

> A **registered service origin** (a verbatim origin minted for a service,
> e.g. `@desired`) MAY publish desired-state on behalf of a target host.
> When it does, it MUST place the **target host id as the first subject
> chunk**, exactly as a proxy producer places its observed device
> ([03-grammar.md §1.6](03-grammar.md), "the observed device as the first
> subject chunk"). The origin remains the service's own concrete identity;
> the target is subject matter, never the origin.

```
<base>/v1/@desired/state/h-3fa9c2d41b7e/config/eth0/desired
```

This is grammar-legal with **zero new mechanism**: it is the §1.6
proxy-producer rule (origin = the machine that publishes; observed subject
= first chunk) applied to a *desired-state author* rather than a device
observer. It is the concrete spelling of the escape hatch decided in
[12-open-questions.md §3](12-open-questions.md), reachable over RPC from
[05-control-rpc.md §3](05-control-rpc.md), and it is the one sanctioned
exception to the "data planes are strictly producer→consumer" rule
([04-planes.md §2 R6](04-planes.md)): the *producer* here is a controller,
the *consumer* the target that reconciles. Its ACL grant is a single
put/delete rule on the service origin's own subtree
([09-operations.md §3](09-operations.md)).

## 4. Why planes and not payloads

The alternative — riding bulk/media on the data classes with a "big"
payload type — fails all three constraints these planes exist for:

- **Selector safety**: `…/h-xxx/**` (a UI's per-host subscription) must be
  affordable on a constrained link; one camera behind it must not turn it
  into a video feed. Verbatim chunks make reaching a *placed* frame
  impossible for any data selector — and registry review is what
  guarantees frames are placed here (the theorem/precondition split of
  design property D2, [03-grammar.md §4](03-grammar.md)).
- **Storage safety**: class-driven storage selectors
  ([04-planes.md §4](04-planes.md)) must never ingest frames or chunks into
  a time-series backend by accident.
- **Different delivery physics**: media wants newest-only/drop; blobs want
  pull/resume/verify. Neither is pub/sub state or telemetry; forcing them
  into data classes would corrupt the class semantics that everything else
  relies on.

## Appendix A — `fanout` (experimental)

*Demoted from §2.2's normative endpoint table in v1.17. The design is kept
on record; the endpoint is experimental: an implementation MAY offer it
behind a feature gate, a registry MAY accept its declaration
([08 §2](08-registry.md)), and no conformant consumer is required to speak
it.*

The demotion is an observation, not a redesign. After a full release cycle
of v1.7's table, adoption was zero: no producer declared `fanout`, no
consumer enabled the reference client's feature (every registry entry that
declares a blob tier excludes it, with the reason written in the TOML), and
the one genuinely one-to-many stream in the deployed fleet chose `@media`
instead. A normative endpoint table should describe what is served;
keeping an endpoint every declaring producer excludes makes the table
aspirational, and aspirational normative text is how a second implementer
ends up building something nobody will speak to. Demotion costs nothing to
reverse: if a real one-to-many customer appears — firmware rollout is the
plausible one — promotion back into §2.2 is a one-line amendment.

The design, unchanged from v1.7: `<id>/fanout` is a *publication* — the
one-to-many case where N consumers each pulling the same bytes is
precisely the amplification this plane exists to avoid, so the producer
publishes once and late joiners recover from the publisher's cache. It is
a declared publisher on the producer's concrete origin like any other
(§3), at the bulk QoS of §2.6, and it carries §2.6's obligation on the
publish side plus blocking congestion control: shedding slices under local
backpressure would trade a bounded delay for an unbounded recovery.

An implementation that does offer it MUST follow the same framing
discipline as every queryable reply in the plane — version-first structs,
a declared encoding tag, and tag-checked-before-decode — rather than
relying on decode failure to reject foreign samples. (The reference
client's wire v3 brings its `fanout` frames up to exactly this rule.)
