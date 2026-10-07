# zk2: the zenkey redesign

zk2 is the next zenkey: a small, typed, introspectable **interface-contract
and reflection layer over Zenoh**. It serves control, perception, mission,
payloads, hardware abstraction, simulation and ground segments, and also
supervision. v1 (the RFC set in `rfcs/`, frozen at v1.50) is the keyspace of
a fleet-observability product; zk2 keeps v1's hard-won lessons and drops its
observability shape.

**Status: in design.** The work is tracked by epic
[#585](https://git.marcpardo.eu/marcpardo/zenkey/issues/585) in milestone
`zk2`.

## The record, in order

| File | What it is |
|---|---|
| [`brief.md`](brief.md) | The original redesign brief. |
| [`r1-analysis.md`](r1-analysis.md) | Part 1: the review request sent to the brief's author. Part 2: **r1**, an independent analysis of the brief against the v1 repository and prior art. |
| [`r1-review.md`](r1-review.md) | The brief author's review of r1. Every ruling is kept; the wording is condensed. |
| [`r2.md`](r2.md) | **r2**: r1 revised by that review. Adds the `zk2/` grammar major, exclusive-by-default ownership, normative state timestamps, hash-verified contract retrieval, behavioural profiles. |
| [`architecture.md`](architecture.md) | **The design of record: r3.3.** r3 redesigned the data plane from twelve use cases. r3.1 is the issue review (decisions, sourced corrections). r3.2 is the check against the three adopters (ZenSight, zenoh-modem, tcgui). r3.3 folds in the gaps found by mapping their contracts (D1–D25). |
| [`spike-report.md`](spike-report.md) | **The spike report** (in progress): the environment, the decisions table that #605 settles, and one section per spike, filled by its issue. Raw data is in [`spike-results/`](spike-results/), copied from branch `zk2-spike`. |

## Rules for this directory

- **The r-number moves only through a reviewed revision.** Every revision
  carries a "what changed since rN" table, as r2 and r3 did.
- **Point releases keep section numbers.** r3.x keeps r3's section numbers,
  so issue citations of the form `r3 §x` stay valid until r4. r4 comes from
  the spike report (#605).
- **The spike's code is throwaway.** It lives on branch `zk2-spike` and is
  never merged. Its results and `spike-report.md` land here by docs PRs.

## v1

**Patch flow for the v1 line:**
1. Fix on a branch cut from `v1`, and open the PR against `v1`. CI runs there.
2. The maintainer tags `0.14.N` on `v1`. The release lane triggers on bare
   tags, whatever the branch.
3. Dispatch the crates lane with `ref: "0.14.N"`, never `main`.
4. Production source builds track `--branch v1`, or pin the latest `0.14.N`
   tag.

- **Frozen.** The v1 convention is frozen at RFC v1.50: errata only.
- **Maintained.** v1 is maintained on the `v1` branch, released as 0.14.x
  patches (#587).
- **Pinned on `main`.** Until each tool is ported to zk2, `main` builds the v1
  tools against the v1 libraries pinned from crates.io: the strangler layout
  (#615).
