# zk2 profiles

A **profile** is an independently versioned specification built on the
core ([`../core.md`](../core.md) §10). The core never depends on one. Each
profile has a directory here, because an implementation reads only `spec/`
and `examples/zk2/`.

## Index

Each row gives a profile's text version, its status (the process below
says what each means), who uses it, and the core version its text is
written against.

**Used by** names the adopter mappings in [`examples/zk2/`](../../examples/zk2/)
that use the profile and, after a semicolon, the shipped services that
do, or reads `none`. A mapping or a service uses a profile where an
instance's descriptor would list it (core §3.3): a contract it implements
lists the profile in `uses`, it implements the profile's standard
contract, or the profile mints its system. The walkthrough
(`examples/zk2/walkthrough/`) is the design's, not an adopter's, and the
tools read every profile, so neither counts.
`zenkey-model/tests/profiles.rs` computes the column from the repository,
and fails when a row's is stale.

| Profile | Text | Status | Used by | Core | Extension points (core §10) | What it is |
|---|---|---|---|---|---|---|
| [`hostid.v1`](hostid/v1.md) | 0.9 | draft | zensight, zenoh-modem, tcgui; zenwatch | 0.40 | 4 only: derivation-only | A system name minted from the machine id, so that one host is one system across every zk2 application (core §1.5) |
| [`freshness.v1`](freshness/v1.md) | 0.5 | draft | zensight, zenoh-modem, tcgui; zenwatch | 0.22 | 2: `freshness.ttl_s`; 4 through `uses` | A resource's staleness horizon: the owner's re-puts, how a reader ages a value, and fresh, stale or unobservable (core R7, S6) |
| [`health.v1`](health/v1.md) | 0.6 | draft | zensight, zenoh-modem, tcgui; zenwatch | 0.23 | 1: [`health.v1.toml`](health/health.v1.toml); 4 through `uses` (`freshness.v1`) | How well a service serves: a status in three levels, its checks and its faults; healthy, unhealthy, stale, or unestablished, and never "absent means failed" (core §4.3, §8.2, R7) |
| [`config.v1`](config/v1.md) | 0.8 | draft | zenoh-modem | 0.28 | 1: [`config.v1.toml`](config/config.v1.toml); 4 through `uses` (`freshness.v1`) | Changing a running service's configuration without locking it out: groups with classes, a change that can cut the link applied only under a window and undone once unless confirmed, persistence as a separate act, and the compact status a constrained face can afford (core §5, §6, O7) |
| [`link.v1`](link/v1.md) | 0.10 | draft | zenoh-modem | 0.32 | 2: `link.exposure`; 4 through `uses` | What crosses a constrained face: a deployment's face policy, one per router, and the contract's exposure, compiled into named denies, one downsampling rule per concrete key, the attachment, window floors and liveness evidence; `@zk` never crosses, and no other principal's credentials get across it (core §8.5, R7, S3) |
| [`alarms.v1`](alarms/v1.md) | 0.10 | draft | zensight | 0.34 | 1: [`alarms.v1.toml`](alarms/alarms.v1.toml); 2: published empty; 4 through `uses` (`freshness.v1`) | The active alarms of one service, one state key per alarm, keyed from its document by ZenSight's recipe: raised, escalated, refreshed and resolved, the state read from the document; confirmed every 450 s while firing, so a hung owner's alarms read stale and an absent one's last-known, and "nothing fires" only from an owner's alarms known whole (core S2, S6, O5) |
| [`telemetry.v1`](telemetry/v1.md) | 0.7 | draft | zensight, zenoh-modem | 0.35 | 1: a standard schema, [`telemetry.v1.json`](telemetry/telemetry.v1.json); 2: `kind`, `unit`, `buckets`, `semantic`; 4 through `uses` | What a metric is: a member's kind and unit, declared or carried by the Point, whose types are the profile's by their artifact's id; whether a sample conforms; a counter's or a histogram's epoch, in-band or from presence, a decrease within an established one the finding and one without evidence a reset; a counter's rate over one epoch; absent is not zero (core §1.5, §7.1, O5) |
| [`desired.v1`](desired/v1.md) | 0.11 | draft | zensight | 0.39 | 1: a standard schema, [`desired.v1.json`](desired/desired.v1.json); 2: `target_param`; 4 through `uses` | A document an author keeps as its own state, keyed by its target: the target binds its own slice by R2, orders versions by its author's stamp, acts on each once per run and never on an older one, refuses whole what it cannot take with the previous kept in force, reverts on a delete and never on silence, and reports what is in force and what it last did in the standard `Report`; the author writes only on a change, and restarts from its record (core S1–S7, §4.2, §4.4, R2) |
| [`redundancy.v1`](redundancy/v1.md) | 0.8 | draft | zensight | 0.40 | 2: `election`; 4 through `uses` | One instance serves a governed interface, the others stand by: a candidate's claim is its instance token, the incumbent the holder of the interface's token, a standby marked in its descriptor and exposing nothing, so a call finds none; a live incumbent wins, the lowest id of a complete read breaks a tie, a vacancy is a withdrawal confirmed by a complete read, and a healed split leaves the lower id; consumers follow the incumbent's stamps; `external` names an actuator's own lock; eventual single writer, never a fence (core §5.1 O1, §6, §8.1, §8.2) |

### Experimental: no adopter yet

No adopter's mapping and no shipped service uses these yet. Their texts,
fixtures, runtimes and tools are as complete as the others', and only
the walkthrough exercises them. A profile leaves this table when one
does (the process, below).

| Profile | Text | Status | Used by | Core | Extension points (core §10) | What it is |
|---|---|---|---|---|---|---|
| [`timing.v1`](timing/v1.md) | 0.7 | experimental | none | 0.55 | 2: `period_ms`, `deadline_ms`, `lifespan_ms`; 4 through `uses` | A stream's cadence, its deadline and its lifespan: the owner publishes each member at its period and no runtime repeats a sample; a reader reports a member missed when more than its deadline passes between two arrivals on its own monotonic clock, seen within max(deadline/50, 1 ms), and met at its next, a member never heard judged only when known and its owner present, and a miss never "down"; a sample arrives expired by its owner's stamp only against a clock bound ε the deployment states, never the HLC's delta, and with none nothing is discarded (core §2.7, §4.1, §8.1, R6) |
| [`arbitration.v1`](arbitration/v1.md) | 0.5 | experimental | none | 0.56 | 2: `policy`, on a requirement; 4 through `uses` | One commander at a time: an arbitrated role follows the first provider, in its binding's order, whose stream arrives within its `timing.v1` deadline and whose interface token it has not heard withdrawn, and holds the dead-man when none is left, the safe value the application's; a withdrawal makes a provider ineligible at once, never absent, and only its token or a read re-admits it, never a sample; no hysteresis; a move of the consumer's own watch hands no authority over; the source from the key, never the payload (core R1, R3, R6, §8.1, §8.2) |

## Layout

```text
profiles/
  README.md               this file: the index, the template, the process
  .history/               published revisions of the profiles' standard contracts (health.v1, config.v1, alarms.v1)
  <name>/
    v<major>.md           the normative text of one wire major
    CHANGELOG.md          the versions of that text
    scenarios.md          the rules a fixture cannot check
    conformance/          fixtures; each file states its format in its description
    <name>.v<major>.toml  the standard contract, for a profile that defines one
    <name>.v<major>.json  the standard schema, for a profile that defines one (core §10 point 1: telemetry.v1, desired.v1)
```

## The template

Every profile text follows this shape, so that a reader finds the same
thing in the same place.

- **A header** gives:
  - the text version, `0.N`;
  - the status: `experimental`, `draft` or `accepted`, as the process
    below says;
  - the core version the text is written against;
  - the profiles it uses, or "none".
- **§0, conventions.** By reference to core §0: the keywords, the roles and
  the evidence tags. A profile adds terms of its own here. In a profile:
  - `[F: <file>]` names a fixture in the profile's own `conformance/`;
  - `[Sc: §n]` names a section of its own `scenarios.md`;
  - a citation of the core's evidence says so, as in
    `[F: core descriptors/ok-derivation-profile]`.
- **§1, the extension points it uses.** All four of core §10 are stated,
  each with how the profile uses it, or "none". A profile that uses point 4
  alone is **derivation-only** (core §10).
- **§2, the rules,** numbered §2.1, §2.2 and so on, and cited that way
  (`hostid.v1 §2.4`). Every MUST and MUST NOT cites `[F:]` or `[Sc:]`, as
  in the core.
- **§3, the contract:** the standard contract it defines (point 1), or
  "none".
- **§4, the vocabulary:** its annotation keys (point 2), each with its type
  and meaning, or "none". A published vocabulary replaces the interim table
  of core Appendix D for that profile.
- **§5, what a tool may conclude.** One row per question a tool answers
  with the profile, in three states:
  - *yes* and *no*, each with the evidence that establishes it;
  - *unestablished*: **not asked** (nobody put the question) or
    **unobservable** (the question was put, and the observation could not
    be had, with why), never folded into *no* (core O5; the judgement
    shape is [`../observer.md`](../observer.md) §1's);
  - **not this profile's**, where the question's premise does not hold
    (an address whose system the profile did not mint);
  - the **polarity**: which pole, if either, is the finding.
- **An appendix against v1:** what is new, what deliberately is not, and
  what v1 had that is not ported. Further appendices as the profile needs.

## The process

- **The text's version** `0.N` moves through the profile's own
  `CHANGELOG.md`, amendment-style. Each entry records what changed, and
  what deliberately did not.
- **A text's status** is one of three:
  - `experimental`: no adopter's mapping and no shipped service uses the
    profile, and the index's **Used by** reads `none`. Its text, fixtures,
    runtime and tools are kept as any other profile's, and the index lists
    it in its second table, "Experimental: no adopter yet";
  - `draft`: an adopter's mapping or a shipped service uses the profile,
    and the maintainer has not accepted the text;
  - `accepted`: the maintainer has accepted the text, as the core's 0.1
    was.
- **A profile leaves `experimental`** when an adopter's mapping
  (`examples/zk2/zensight/`, `zenoh-modem/` or `tcgui/`) or a shipped
  service (`zenwatch`) uses it. Its row then moves to the index's first
  table, and its text becomes `draft`. A profile whose last user stops
  using it goes back to `experimental`.
- **A status moves without a text version.** The profile's `CHANGELOG.md`
  records the move in an entry of its own, headed by the new status and
  the date, which says that no rule changed. The text's header and the
  index's row move with it. `zenkey-model/tests/profiles.rs` keeps them
  agreeing: it fails when a row's **Used by** is stale, when a profile
  nobody uses sits outside the experimental table or a used one inside
  it, and when a row's text version, status or core version is not its
  text's header's.
- **No new tier before an adopter asks.** A new profile, among them the
  tiers still planned (media and views, blob and jobs, clock), is written
  only once an adopter's mapping needs it, so that it enters the index as
  `draft`, already used. `media.v1` is first in line: ZenSight's mapping
  names it in `zs.parallax.v1`'s `uses`, for the camera frames an explicit
  `@stream` carries meanwhile.
- **The file name carries the wire major** (`v1.md`). A change that would
  change what a conforming participant puts on the wire, or the values it
  derives, is breaking. A breaking change is a new major, `v2.md` beside
  `v1.md`, with its own text version, never an amendment of `v1.md`.
- **Fixtures land with their rule.** A machine-testable rule arrives with
  its fixture or scenario, in the same change.
- **A standard contract** goes in the profile's directory, and is
  published in `spec/profiles/.history/` (core §9.7), append-only, like
  `examples/zk2/.history`. After an intended change, publish the new
  revision:

  ```bash
  cargo run -p zenkey-model --bin zk2 -- contract bundle spec/profiles/<name>/<name>.v<major>.toml --history spec/profiles/.history
  ```

  A contract that moves here from elsewhere keeps its published revisions
  where they were, and is copied here byte for byte (core §9.7), as
  `health.v1` and `alarms.v1` were from `examples/zk2/.history`.
- **A standard schema** (core §10 point 1) goes in the profile's
  directory, and has no `.history` of its own: the text publishes each
  revision's artifact id (its §3), every one of them staying the
  profile's, and a fixture pins them (`telemetry.v1`'s `schema.json`).
  The contracts that list it are published in their own histories, so a
  change to it republishes each of them, compatible or not as core §9.8
  classifies it. `telemetry.v1`'s moved here from
  `examples/zk2/zensight/schemas/`, and the four ZenSight contracts list it
  from here.
- **A profile that needs the core to change** says so, and the core
  changes first, through [`../CHANGELOG.md`](../CHANGELOG.md), which
  records the version each change made. What each profile needed:
  - `hostid.v1`: that an instance can declare a profile no contract
    uses, and, for its runtime, that a binding can name a provider on the
    service's own, minted, system;
  - `freshness.v1`: that W105 reads its published vocabulary, and that a
    re-put is a mutation;
  - `health.v1`: that a clock ahead is reported by a stream, which the
    guard does not hold, rather than by its status;
  - `config.v1`: no change, since core §7.3's `oneOf` expresses its
    discriminated replies;
  - `link.v1`: that Appendix D publishes its vocabulary, `exposure`, and
    drops the interim `downsample_ms`, the rate being the face policy's.
    Then that §8.5 says what a south region's declaration grant covers:
    every queryable as its owner declares it. Keeping every other
    credential off a face, and reading two revisions of one interface
    together, needed no change: they rest on what core §11.2, §11.3 and
    §9.8 already say;
  - `alarms.v1`: that Appendix D publishes its vocabulary empty and drops
    the interim `severity_default`, every document carrying its severity;
  - `telemetry.v1`: that a profile's first point can be a standard
    schema, a file whose types are the profile's by their artifact's id,
    and that Appendix D publishes its vocabulary;
  - `desired.v1`: that Appendix D publishes its vocabulary,
    `target_param`, and retires the interim `target`. Its author's restart
    needed no core change: it puts its documents again from its new
    instance, as `alarms.v1`'s owner does, and its targets adopt them
    without an act. A stamp's id that is never 0 needed none either. That
    rule is `desired.v1`'s own, in its §2.4;
  - `redundancy.v1`: that a descriptor marks a standby (`"standby":
    true`, D012 when tokenless), and that the core defines what a standby
    holds and exposes. Also that a standby taking over and an incumbent
    stepping down follow §8.2's steps and their reverse, that the
    tokenless set never names a governed interface, that the split-brain
    check reads the mark, and that Appendix D publishes its vocabulary,
    the interim `election` unchanged. Then that §8.1 says what its runtime
    measured. A session's read can answer its own tokens itself, even
    through a router that refuses the read, and a complete read covers
    only what its routers' clients and its linked peers hold;
  - `timing.v1`: that Appendix D publishes its vocabulary, the interim
    row unchanged, and that §4.1 says a session moves its HLC to a later
    stamp it receives, which a lifespan's clock bound must cover. Also
    that Appendix B says a stamp's time counts from the Unix epoch. Then
    that §4.1 says a node with an HLC also stamps an unstamped put it
    routes, its own subscriber's included, which a reader with one never
    sees unstamped;
  - `arbitration.v1`: that a binding is an ordered list whose order a
    descriptor keeps and a profile may read, that a reader may act on a
    withdrawal heard live where acting concludes no absence, and that
    Appendix D publishes its vocabulary, the interim `policy` unchanged.
- **Lessons go to guides, not MUSTs,** as in the core.

## How the harnesses take profiles in

- **The reference.** `zenkey-model/tests/profiles.rs` runs every file under
  `spec/profiles/*/conformance/`. An unknown file fails the test, so a
  fixture cannot land unrun. A `README.md` there is prose.
  - To regenerate the expectations after an intended change, run the
    command below, then read the diff: a bless claims that every changed
    expectation is right.

    ```bash
    ZK2_BLESS=1 cargo test -p zenkey-model --test profiles
    ```
  - The same test computes the index's **Used by** from the repository,
    and checks each row against it and against its text's header
    (`the_index_says_who_uses_each_profile`, whose doc comment lists what
    it reads).
  - The same test walks every standard contract,
    `spec/profiles/<name>/<name>.v<major>.toml`: it loads with no finding,
    is named by its interface id, and is published in
    `spec/profiles/.history`, compatible with every earlier revision there.
  - A profile's session-free code lives in `zenkey-model` (`hostid`,
    `freshness`, `health`, `config`, `link`, `alarms`, `telemetry`, `desired`,
    `redundancy`, `timing`, `arbitration`), and its runtime in
    `zenkey`. Its scenarios run as tests of the runtime, one test per
    section, named after it, as the core's do. A scenario that is a tool's
    runs as a test of that tool.
- **Other implementations** run the same files. Each fixture states its
  format in its `description` member, as the core's fixtures do (core
  Appendix E). The second implementation (`impl/python/`) takes in each
  profile as a family of its own, reading
  `spec/profiles/<name>/conformance/`.
