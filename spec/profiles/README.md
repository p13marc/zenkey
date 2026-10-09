# zk2 profiles

A **profile** is an independently versioned specification built on the
core ([`../core.md`](../core.md) §10). The core never depends on one. Each
profile has a directory here, because an implementation reads only `spec/`
and `examples/zk2/`.

## Index

| Profile | Text | Status | Core | Extension points (core §10) | What it is |
|---|---|---|---|---|---|
| [`hostid.v1`](hostid/v1.md) | 0.2 | draft | 0.20 | 4 only: derivation-only | A system name minted from the machine id, so that one host is one system across every zk2 application (core §1.5) |
| [`freshness.v1`](freshness/v1.md) | 0.1 | draft | 0.21 | 2: `freshness.ttl_s`; 4 through `uses` | A resource's staleness horizon: the owner's re-puts, how a reader ages a value, and fresh, stale or unobservable (core R7, S6) |

## Layout

```text
profiles/
  README.md               this file: the index, the template, the process
  .history/               published revisions of the profiles' standard contracts (none yet)
  <name>/
    v<major>.md           the normative text of one wire major
    CHANGELOG.md          the versions of that text
    scenarios.md          the rules a fixture cannot check
    conformance/          fixtures; each file states its format in its description
    <name>.v<major>.toml  the standard contract, for a profile that defines one
```

## The template

Every profile text follows this shape, so that a reader finds the same
thing in the same place.

- **A header** gives:
  - the text version, `0.N`;
  - the status, `draft` or `accepted`;
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
  alone is **derivation-only** (core §10, 0.19).
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
    be had, with why), never folded into *no* (core O5);
  - **not this profile's**, where the question's premise does not hold
    (an address whose system the profile did not mint);
  - the **polarity**: which pole, if either, is the finding.
- **An appendix against v1:** what is new, what deliberately is not, and
  what v1 had that is not ported. Further appendices as the profile needs.

## The process

- **The text's version** `0.N` moves through the profile's own
  `CHANGELOG.md`, amendment-style. Each entry records what changed, and
  what deliberately did not. A text is `draft` until the maintainer
  accepts it, as the core's 0.1 was.
- **The file name carries the wire major** (`v1.md`). A change that would
  change what a conforming participant puts on the wire, or the values it
  derives, is breaking. A breaking change is a new major, `v2.md` beside
  `v1.md`, with its own text version, never an amendment of `v1.md`.
- **Fixtures land with their rule.** A machine-testable rule arrives with
  its fixture or scenario, in the same change.
- **A standard contract** goes in the profile's directory, and is
  published in `spec/profiles/.history/` (core §9.7), append-only, like
  `examples/zk2/.history`.
- **A profile that needs the core to change** says so, and the core
  changes first, through [`../CHANGELOG.md`](../CHANGELOG.md).
  `hostid.v1` needed 0.19, so that an instance can declare a profile no
  contract uses, and its runtime 0.20, so that a binding can name a
  provider on the service's own, minted, system. `freshness.v1` needed
  0.21, so that W105 reads its published vocabulary and a re-put is a
  mutation.
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
  - A profile's session-free code lives in `zenkey-model` (`hostid`,
    `freshness`), and its runtime in `zenkey`. Its scenarios run as tests
    of the runtime, one test per section, named after it, as the core's
    do. A scenario that is a tool's runs as a test of that tool.
- **Other implementations** run the same files. Each fixture states its
  format in its `description` member, as the core's fixtures do (core
  Appendix E). The second implementation (`impl/python/`) takes in each
  profile as a family of its own, reading
  `spec/profiles/<name>/conformance/`.
