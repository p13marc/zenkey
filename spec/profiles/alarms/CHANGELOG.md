# `alarms.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.2 — 2026-10-20: the cold reader's findings, and what the runtime settled (#800, AB; #609)

Two readings of 0.1 met here. The spec-only implementation read it cold
(zk2py round 38): every fixture passed, 281 checks of `alarms.v1`'s 1715,
and it filed seven findings, F-126 to F-132. The runtime (`zenkey::alarms`,
#800's AB) puts what the text asks of an owner over the standard contract,
embedded, and runs `scenarios.md` §1 to §7 on in-process routers. Building
it found five more places where 0.1 left an owner to infer. Written against
core 0.34, unchanged.

Each answer is stated in the text, and pinned by a fixture case where a
fixture can pin it. One answer adds to the model, F-126's `decode`, and one
changes it, F-130's window across a face. No case that existed before
changes: the new cases were written by hand and pass as written. The
evidence is now `documents.json`, 58 cases, `transitions.json`, 27, and
`conclusions.json`, 95; `keys.json` is unchanged, at 75.

**Answered: the seven findings.**
- **F-126, how a payload's bytes become a document** (§2.4). 0.1 judged a
  JSON value, and left the bytes to the parser, so a duplicate member, a
  non-finite number or a lone surrogate escape read as each parser chose:
  two readers could read two severities from one put, or derive two keys.
  - Now a reader reads a payload as core §3.3's D000 reads a descriptor:
    strict JSON (UTF-8, RFC 8259 with nothing after the value, no
    duplicate member at any depth, every number finite, every string with
    a UTF-8 form). Anything else is undecodable. An owner's MUST to put
    documents of the schema now says "as strict JSON".
  - §2.2's "whose names are distinct" now holds by that rule.
  - The reference had no reading of bytes at all: it read decoded values.
    `zenkey_model::alarms::decode` is now one, through the model's strict
    JSON, as bundles and descriptors are read; a refusal is
    `Invalid::Json`, token `json`. zk2py's reading stands.
  - `documents.json`: nine cases given as their `text`. A duplicate
    severity, a duplicate label name, a duplicate member the schema does
    not name, `NaN`, `Infinity`, a lone surrogate, and text after the
    value are `json`; a document, and a surrogate pair, read.
- **F-127, `labels` absent and `labels: {}`** (§2.6). Equal. Neither has a
  label, both derive one key and say the same, and an owner that omits an
  empty map and one that writes it put the same document: read as
  unequal, a reader would count a refresh no owner made.
  - That is the reference's reading: its document type reads an absent
    `labels` as an empty map. zk2py's reading changes.
  - Any other member absent is equal only to itself absent. An empty
    `category` is a value a tool shows and filters by, so `category: ""`
    is not an absent one, and its change is a refresh.
  - `transitions.json`: three cases, the two orders of `labels` and an
    empty `category`.
- **F-128, what a GET after a loss heals** (§2.14 step 4, §2.15). Reading
  1, the narrower one, the reference's and zk2py's: step 4 holds until a
  GET has answered **this key**, with a value or a `reply_del`. A GET that
  drew nothing for the key, complete or not, heals nothing for it: an
  empty reply set is no verdict (core O5), as §2.15's `no_member` already
  said of a whole owner.
  - §2.15 now says over which span a window is lossless: from its
    declaration, or, after a loss, from the first complete GET that
    followed it, a reader's re-seed (core §4.2). A loss before that GET is
    healed by it, one after it is not, and `window`'s 900 s count over that
    span. The model takes `lossless` and `listened` as given, and its
    `Window` doc says so.
  - `conclusions.json`: the case zk2py named, a loss, a put 10 s old and a
    complete GET that drew nothing for the key: `incomplete`.
- **F-129, an earlier instance told by a stamp's id** (§2.14 step 5). The
  step now compares the id with the present descriptor's `meta.zid` by
  value, and says what each of the three cases reads.
  - **Another session's stamp,** a router's re-stamp or any break of S1,
    reads `previous_instance` too: not asked, shown as last-known. It is
    also core §4.2's foreign stamp, a finding a tool reports apart.
  - **No `meta.zid`:** a stamp is unattributable, never foreign (core
    §3.3), and the reader goes on to step 6.
  - **A re-mint that keeps its session** keeps its zid, and is not told
    apart. §2.9 says so too: the owner still puts every alarm again at a
    re-mint, which keeps its GET answers its own, and the reference's
    re-mint (`Service::new_epoch`) does, before its new descriptor and
    tokens. Core §1.5 already asks a new session, a SHOULD.
  - No fixture: `previous_instance` is the reader's input, as it was.
- **F-130, the owner verdict's reasons, a face, and their order** (§2.15).
  The verdict is now an ordered list, the model's order and zk2py's:
  §2.14's steps 1 and 2 with their reasons (`presence_incomplete`,
  `no_descriptor` and `face_closed` included, which 0.1's fixture had and
  its text did not); the seed (`no_seed`, `seed_incomplete`); the window
  (`window_lost`, then `owner_gap`); `answered`; across a face, an empty
  GET (`across_face`); `no_member` and `window_short`; then `window`.
  - **"Present and listed"** now reads "§2.14's steps 1 and 2 do not
    answer", so an owner across a face that does not close the alarms goes
    on, the deployment's word standing for its descriptor. 0.1's fixture
    already established one by `answered`.
  - **A window across a face** is not asked whether its owner was present
    throughout: presence does not cross. **A change of the model:** it
    read `owner_gap` there, so a reader that said "not known present" made
    every owner across a face with a subscription unobservable, `answered`
    included, which the text and the fixture's `answered` case rule out.
    0.1's across-face case carried `present_throughout: true`, and passes
    either way.
  - `conclusions.json`: six cases. Across a face: `answered` with a window
    not present throughout, an empty GET with such a window, and an empty
    GET with none (`across_face` before `no_member`). And the overlaps: a
    timed-out GET with a lossy window (`seed_incomplete`), a lossy window
    not present throughout (`window_lost`), presence possibly incomplete
    with no GET (`presence_incomplete`).
- **F-131, which owners are in scope** (§2.15). The services a selector
  selects that the reader knows of: those its presence read finds
  (§2.13), and those it knew of before, by an earlier reading of its own
  or an archive's keys (core S6). One known only the second way, which a
  complete presence read does not find, is absent, not asked; a tool says
  which owners it asked and how it knew of each. `scenarios.md` §8 says
  how T knows of `lab/svc`: its own step 1. No model change: `any_firing`
  counts the owners its caller gives.
- **F-132, the third question's reasons** (§5). The row now names
  `no_document` for a delete and `undecodable` for a payload that does not
  read, as `conclusions.json` expected, and the fixture's description
  names them.

**Stated: the round's other guesses,** each the model's reading already.
- At §2.14's step 6, a freshness that `freshness.v1` does not ask
  (observations that contradict the latest sample) reads unobservable,
  with its reason, as an unobservable one does.
- §5's last question reads a put that does not read as `undecodable`,
  whatever its freshness.
- `scenarios.md` §5 spells out the readings §2.6 decides: R's and P's
  deletes read retired, since S held neither, and B's unchanged re-put
  reads confirmed, before its resolved and retired.

**Stated: what the runtime settled.**
- **A duplicate observation is no act** (§2.5). The condition found again
  at the severity and with the content held: the owner puts nothing, or
  the same document, which confirms it. `scenarios.md` §1 already expected
  either; §2.5's table had no row for it. The reference puts nothing.
- **An evaluation confirms again** (§2.7). An owner that stopped
  confirming an alarm it no longer evaluated, and evaluates it again,
  confirms it again. The reference turns an alarm's confirmation back on at
  any act on it.
- **The work's order at a restart, and the bound** (§2.9). 0.1's order
  decides what each document found gets. The owner deletes and resolves
  before it adopts, so that a key it retires takes none of the bound's
  room. Past the bound, it adopts up to it and neither resolves nor
  deletes the rest: a resolve would say their condition cleared, and
  §2.5 forbids a firing alarm's delete outside a restart's retirements. A
  reader that held one reads it as the previous instance's last word,
  stale within 900 s. The reference adopts in key order and names the rest
  in its report.
- **What the record holds** (§2.9, a SHOULD). Each alarm's latest
  document as put, by its key, a resolved one included until its delete.
  Without that, a process that ends between a resolve's put and its delete
  would have its restart put the alarm firing again after its resolve,
  which a reader reads as a raise under an old `timestamp`; with it, the
  restart finds the alarm resolved and deletes it (step 1). The reference
  saves the record after every put and every delete.
- **An owner's puts are strict JSON** (§2.4, with F-126).

**Deliberately not stated.**
- **The record's format.** It is the owner's own, as 0.1 said. The
  reference's (`zenkey::alarms::FileRecord`) is a JSON file, `{"profile":
  "alarms.v1", "alarms": {<alert key>: <document>}}`, and the record seam
  is the application's (`zenkey::alarms::Record`).
- **A delete that fails after a resolved put.** §2.5's MUST bounds it by the
  tombstone window. The reference holds the alarm resolved and unconfirmed,
  and deletes it at its next resolve; when to retry is an implementation's.
- **§2.8's report of a refusal** stays a SHOULD on the owner: the reference
  returns core §2.7's refusal to its writer, naming the bound and whose it
  is, and leaves the `health.v1` check to the application.
- **No change to `freshness.v1`.** §2.7's "evaluator stuck" is a case of
  `freshness.v1` §2.4's "a value it no longer vouches for", which that text
  words as "the source it reports went away"; its wording is
  `freshness.v1`'s to widen.

## 0.1 — 2026-10-20: the first text, draft (#800, AA)

`alarms.v1` is the profile of #613 that zenwatch's `alerts`, the
watchdog's `alert-firing` and ZenSight's alerts wait on
(`docs/zk2/migration.md`). Its standard contract was written with
ZenSight's mapping (#622), published as revision 1.0
(`sha256:95579151…`), and carried ZenSight's key recipe and lifecycle in
its comments. This text gives it rules, and is written against core 0.34,
which publishes its vocabulary, empty.

**Decided on #800, before the text** (the maintainer's, not reopened
here). Where the published revision 1.0 already answers a question, the
text makes its answer normative.
- **One key recipe, fixed by the profile,** byte for byte ZenSight's: the
  rule and the discriminating labels framed, FNV-1a 64, 16 lowercase hex
  digits; `host` and `host.*` excluded first (D23); ZenSight's sanitizing
  fallback stated exactly; the system and the service never hashed.
- **The lifecycle:** a raise, an escalation with a new `timestamp`, a
  refresh that keeps it and sets `observed_at_ms`, a resolve as a resolved
  put then a delete, a re-raise with a new `timestamp`. The state is read
  from the document, never the sample's kind, and a bare delete reads as
  resolved.
- **Freshness:** `freshness.ttl_s = 900` kept; each firing alarm re-put
  within `freshness.v1`'s period, so a present but hung owner's alarms
  read stale, and an absent owner's are last-known (core S6), never firing
  now.
- **No `events` resource** at 1.0.
- **JSON,** revision 1.0 kept byte for byte.
- **A structured reference,** system, service and alert key, never
  dot-packed (D23).
- **Out of scope:** acknowledgements, silences, incidents and impact
  (ZenSight's `zs.catalog.v1`), zenwatch's inhibition, and debounce,
  hysteresis and refresh limits, an implementation's.

**Stated.**
- **The contract moves** under the profile (§3), unchanged:
  `spec/profiles/alarms/alarms.v1.toml` and its JSON Schema, the same
  fingerprint before and after, revision 1.0 copied byte for byte into
  `spec/profiles/.history/alarms.v1/`. The comments and the doc string
  were rewritten, and are outside the fingerprint; the schema, whose
  descriptions are inside it, moved unchanged.
- **The key** (§2.2) and **the sanitizing step** (§2.3), with the test
  vector; **the document** (§2.4); **the lifecycle** (§2.5) and **the
  reading of transitions** (§2.6); **freshness** (§2.7); **the bound**
  (§2.8); **a restart and a clean close** (§2.9); **device-as-service**
  (§2.10); **constrained faces** (§2.11); **references** (§2.12); **the
  tokenless set** (§2.13); and **the reader's procedures**, for one alarm
  (§2.14) and for an owner's alarms (§2.15).
- **What a tool may conclude** (§5): is this alarm firing; is anything
  firing at or above a severity under a selector; does a key re-derive
  from its document; is a firing alarm past its horizon while its owner
  is present. The changes against v1's alerts (Appendix A), and the
  adopters' mappings (Appendix B).
- **Evidence:** `conformance/keys.json` (75 cases: 42 derivations, run
  through ZenSight's own code path and a second transcription before they
  were written, 15 host-scoped names, 7 references and 11 keys parsed),
  `conformance/documents.json` (49), `conformance/transitions.json` (24),
  `conformance/conclusions.json` (88: 40 readings, each answering two
  questions, 20 owners' alarms, 16 scopes, 12 re-derivations), and ten
  scenario sections, seven the runtime's and three a tool's.
  `zenkey_model::alarms` runs every fixture.

**Decided here, where the record was silent.**
- **The label order's tiebreak is the value,** in byte order: names of a
  JSON object are distinct, but two names can sanitize to one, and
  ZenSight's sort is over (name, value) pairs.
- **Sanitizing is the reader's step on every document,** since it changes
  nothing in framed input, and framed input is a SHOULD, not a MUST: the
  reference adopter keys on a sanitized rendering and warns, rather than
  refusing an alarm. The step is not injective, which is the reason for
  the SHOULD.
- **The document's rules:** both times from 0 to 2^53−1;
  `observed_at_ms` never earlier than `timestamp`, and never on a
  resolved document (ZenSight's `resolved()` clears it); an integral
  number read as an integer, as JSON Schema 2020-12 reads one, and
  members the schema does not name ignored. A payload outside the schema
  reads undecodable, never at a guessed severity.
- **Label names and values:** a name SHOULD be lowercase dotted snake
  case (ZenSight's names are), and a value SHOULD be canonical, as core
  §1.4 asks of a slugged value. Every discriminating label is identity,
  so a reading that moves belongs in `summary`.
- **`category` is free text,** shown and filtered by a tool, interpreted
  by no rule.
- **A new `timestamp` is later than the one before it on that key,** the
  previous plus 1 where the clock has not passed it, so that a reader
  tells a transition by a change of `timestamp`.
- **An escalation is a change of severity either way,** as ZenSight's
  `decide` publishes one, and a reader that holds a firing alarm reads a
  new `timestamp` at one severity as a raise it did not see resolved.
- **A resolve's delete comes within the tombstone window** of its
  resolved put, and SHOULD at once. A firing alarm is never deleted
  without a resolved put first, except a key retired at a restart.
- **A confirmation is the document re-put unchanged,** `observed_at_ms`
  included, as `freshness.v1` §2.4 defines a re-put and the reference
  runtime's refresher makes one. A refresh with a new `observed_at_ms`
  confirms too, as any put does. An owner confirms only an alarm whose
  condition it still evaluates, so that a confirmation means the owner
  still sees it.
- **At the bound,** a raise is refused as core §2.7 says, and a firing
  alarm is never resolved or deleted to make room, since a resolve says
  the condition cleared.
- **A restart adopts from the owner's record, else an archive on its own
  side,** the sources core §4.3's catch-up names. ZenSight's adoption GETs
  its own selector, which in v1 a `latest` storage answered; in zk2 no
  storage may (core S4), and the previous process that answered is gone.
  The order (resolved, then phantom or undecodable, then a rule no longer
  evaluated, then adopted) is ZenSight's `adopt_persisted` with its
  known-rules check.
- **A new instance puts again what it keeps,** before it answers a GET
  with it, so that its answers carry its own stamps (core S2), and a
  reader reads an alarm whose latest put an earlier instance stamped as
  that instance's last word, `previous_instance`: not asked, shown as
  last-known. Without that rule, a reader holding a crashed owner's alarm
  would read it firing until it went stale.
- **A clean close leaves the alarms** (a SHOULD NOT resolve), as
  `health.v1` leaves its status: ZenSight resolves all at shutdown,
  because a v1 storage served an abandoned firing document as current for
  ever, and zk2 refuses that storage. Resolving at a close would notify
  twice and undo every acknowledgement at each restart. A service removed
  for good MAY resolve.
- **Device-as-service** as `health.v1` §2.9 has it: each device service
  puts its own, a parent's alarms are its own, and a member is keyed apart
  by a label. D23's `<parent>.<slug(device)>` stays a convention of
  `examples/zk2/shapes.md`: a structured reference splits no service name,
  so nothing here depends on it.
- **Constrained faces:** revision 1.0 declares no `link.exposure`, so its
  alarms are `host`, and cross only as GETs by grant. A face closed to them
  leaves every question unobservable, never "nothing fires".
- **The reference is named in text only.** Adding a type to the schema
  would be a revision no alarm's wire needs, and `zs.catalog.v1` already
  declares its own `AlertRef`.
- **The tokenless set** as `health.v1` §2.7 has it.
- **What a tool may conclude,** in full:
  - **"Is this alarm firing?"** answers yes only from a fresh firing
    document of the present instance; no from a resolved document, a
    delete or a `reply_del`, or a key the owner's established alarms do
    not hold; and never from silence.
  - **A loss** makes the answer unobservable until a GET answers the key:
    a lost delivery may have been a resolve, or a re-raise.
  - **An owner's alarms are established** by a complete S4 GET that it
    answered (core S2: its answer is its alarms), or by a complete, empty
    GET and a lossless window longer than 900 s (every firing alarm is put
    within 450 s), the owner present throughout. An empty reply set alone
    stays no verdict (core O5), and the GET is what tells a hung owner
    from one with nothing firing.
  - **"Is anything firing at or above S?"** is yes on any yes, and no only
    when every owner asked is established and none of its alarms that may
    be at or above S is unobservable; absent owners are counted apart.

**Deliberately not stated.**
- **No `events` resource.** A transition is the stamped change of an
  alarm's state (§2.6); a later minor may add a stream of transitions, as
  architecture §3.12's "plus transitions" foresaw.
- **No core amendment beyond the vocabulary** (core 0.34): the recipe is
  not an extension point, adoption rests on core §4.3, and "no alarm
  fires" on core §2.7's complete readings.
- **No acknowledgements, silences, incidents or impact.** They stay in
  `zs.catalog.v1`, keyed by the structured reference, and zenwatch's
  inhibition stays zenwatch's.
- **No debounce, hysteresis or refresh limit.** When to raise, how long a
  condition must stay clear before a resolve, and how often a refresh may
  go out are an implementation's: ZenSight's `for:`, its recovery window
  and its 30 s refresh floor are lessons for a guide, not MUSTs.
- **No per-application recipe or host-scoped set.** The descriptor has no
  slot for a profile's parameters (ZenSight's G3); one recipe serves the
  one implementer.
- **No severity beyond the three.** A later minor that adds one reads as
  undecodable to a reader built before it: the safe direction.
- **No runtime and no tool here.** `zenkey`'s alarms owner is #800's AB,
  and runs `scenarios.md` §1–§7; zenctl's verbs and zenwatch's `alerts`
  are its AC, and run §8–§10.
