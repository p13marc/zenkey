# `alarms.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

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
