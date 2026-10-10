# zenwatch changelog

`zenwatch` ships as Forgejo release binaries, never on crates.io. A
notifier's config is written once and edited rarely, so this file has to be
complete enough that a v1 config can be moved in one sitting: every row
names where its question went, or that nothing on `main` asks it yet. No
aliases and no shims, except where the two spellings were always one fact
(`base` and `namespace`).

## Unreleased (`main`, zk2) — the read side on zk2 (#614, FL2)

zenwatch's rules, presence, contracts and doctor read a zk2 deployment
through the in-tree engine (`zenkey-fleet` 0.20, by path), on two
sessions: one in the deployment's namespace, one in none (decided
2026-10-08). What the daemon publishes about itself — the health,
`firing/{rule_id}` and doctor documents, the registry they are declared
in — is still v1's until FL3, under renamed dependencies.

### Rules

| v1 | zk2 | Notes |
|---|---|---|
| `origin-down <ORIGIN>` | `instance-gone <SYSTEM/SERVICE>` | refused with that said (exit 2), no alias. Either position `*` (`*/tc`, `host-a/*`). One state **per address**, read from the engine's live presence projection: up while an instance token is held, gone only under a complete read, unobservable otherwise. A pattern's second address is an identity of its own; an exact address is watched from the start |
| `liveliness-gone v1/*/state/*/alive` | `liveliness-gone <NS>/zk2/<SYSTEM>/<SERVICE>/@zk/instance/*`, or `instance-gone` | on the deployment's own zk2 presence tokens the rule rides the projection, one state per thing that persists: an address (`…/@zk/instance/*`, `…/@zk/**`), an address and interface (`…/@zk/alive/…`), an address, interface and member (`…/@zk/member/…`, a handover is no gap) |
| `liveliness-gone <any other SEL>` | the same | the daemon's own ledger, one state per token key, seeded by a liveliness read that says when it ran to its timeout (the v1 roster ask never noticed), re-read after every loss |
| `alerts <SEL>` | — | **dark** until zk2's alarm profile (`alarms.v1`) exists (#613): refused by `check-config` and `run`, exit 2, naming #613 — never parsed into a rule that could never fire |
| `alert-firing <SEL> [<MIN-SEVERITY>]` | — | dark with `alerts` (#613), refused |
| `rate-above`/`rate-below`/`silent-for`/`invalid-payload`/`qos-mismatch` over `v1/…` | the same over zk2 wire keys (`<NS>/zk2/…`) | selectors stay wire keys on the session in no namespace. `invalid-payload` checks a payload against its contract's declared type (spec §7.2, §7.3), `qos-mismatch` against its resource's QoS (§2.4) — contracts from `contracts`/`--contracts` or retrieved from the bus, never a served schema |
| `doctor <CHECK-ID>` (v1 ids) | `doctor <CHECK-ID>` (zk2 ids, below) | `clocks_synced` reaches a `doctor health-*` rule's doctor |
| — | the first look is a baseline | an address already down at start is remembered, not paged one by one, and said once in a `baseline` notification per rule (as the scheduled doctor's baseline) |
| a dropped broadcast: one `unobservable` per `alerts`/`liveliness-gone` rule, every time | the rule's `<rule_id>:observer` goes `unobservable`, then `observable_again` when a complete re-read stands | an address firing stays firing through the loss; one whose stop the loss hid is found by the re-read |

### Configuration and flags

| v1 | zk2 | Notes |
|---|---|---|
| `bus.base` | `bus.namespace` (`base` still spells it) | the deployment namespace the resolved reads open their session in; empty is the bus root |
| `ZENWATCH_BASE` | the same | the namespace's env rung, below the config |
| `bus.registry: [DIR]` | `contracts: [PATH]`, `--contracts PATH` | refused by name (exit 2). An authoring file, a directory of them, or a `.history` root; the flag adds to the config. A revision held offline is never retrieved; every other one comes from its holders (spec §8.4). A context's `registry` is not read |
| `run --context NAME` | `run` and `check-config` `--context NAME` | |
| — | `run`/`check-config` `--namespace NS` (alias `--base`) | flag > config > env > context > empty, as zenctl resolves it |
| — | `clocks_synced: false` (top level) | the operator's word that the clocks agree within the HLC delta (`freshness.v1` §2.6), for the doctor's `health-*` checks, scheduled or a rule's |
| `discipline.inhibit: { enabled, depth }`, on by default | removed | `enabled: true` — or a block without `enabled`, which v1 read as on — is refused, saying it read v1's catalog (`edge/*`, `entity/*`, `alias/*`) and no zk2 profile brings it back. `enabled: false`, or no block, runs |
| `discipline.group_by: ["origin"]` (the default) | `["system"]` (the default), or `address` | `origin` is refused: zk2 keys carry no origin chunk. `system` is the `<system>` of the address a notice is about (a host under `hostid.v1`) |
| `doctor.sample` | — | refused: zk2's deep checks read every owner's state once and take no sample |
| `doctor.deep` (freshness, storage coverage, budgets) | `doctor.deep` | zk2's data-plane checks: `state-stamp-foreign` and `population-over-bound` |
| `check-config` checks the file | the file, the context, the namespace, the contracts and the state file | `run` and `check-config` pass one gate (`load_checked`), so a refusal either makes, the other makes too |

### The scheduled doctor's check ids

zk2's doctor asks different questions; no id was renamed in place. Where a
v1 id's question still has a home, it is named — the nearest question, not
a rename.

| v1 | zk2 | Notes |
|---|---|---|
| `slice-sync`, `schema-drift` | `contract-drift` | providers of one interface at revisions the classifier calls review or breaking (§9.8) |
| `describe-missing`, `describe-totality`, `slice-parse` | `contract-unavailable`, `descriptor-invalid` | a descriptor naming a revision no holder serves verified (§8.4); a descriptor failing its check (§3.3). A contract that does not load is `zk2 contract lint`'s, in CI |
| `introspect-coverage` | `token-missing` | an instance whose tokens disagree with its descriptor (§8.1) |
| `admin-unreachable`, `router-version-skew` | the same ids | |
| `storage-coverage` | `storage-on-state`, `archive-unaligned` | inverted: a router storage on an owner's state is the finding now (§4.2 S4) |
| `unstamped-state`, `timestamp-stamped-elsewhere` | `state-stamp-foreign` (`deep`) | |
| `cardinality-over-declared`, `budget-exceeded` | `population-over-bound` (`deep`) | |
| `stale-state` | — | freshness is `zenctl check conform`'s (`freshness`), not the doctor's |
| `payload-undecodable`, `payload-invalid`, `qos-observed-mismatch` | — | rules: `invalid-payload <SEL>`, `qos-mismatch <SEL>` |
| `unregistered-traffic` | — | no registry: a key's identity is `zenctl echo`'s lens |
| `rate-over-declared` | — | `zenctl check conform`'s `rate` |
| `field-vanished`, `field-stuck`, `field-new` | — | `zenctl field` |
| `kind-mismatch` | — | dark until its profile exists (#613) |
| — | `split-brain`, `binding-unsatisfied`, `presence-over-budget`, `shm-memlock-low`, `health-failed`, `health-degraded`, `health-stale`, `health-inconsistent`, `hostid-duplicate` | new questions |
| a finding absent from the next run: `resolved` | `resolved` only when the run established the finding's check | a check unobservable or not asked has not seen the finding go: it stays standing |
| coverage: producers, introspect, describe, `registry diff` | coverage: namespace, the presence read, routers, `health.v1` and its clock, checks established / unobservable (named) / not asked | |

### The sink payload

| v1 | zk2 | Notes |
|---|---|---|
| `inhibited_by` | — | removed with inhibition |
| — | `presence` | the projection's move that decided a presence notice, as the engine pins it (`zenkey_fleet::report`): `at`, `source` (`event`/`seed`/`dropped`), `address`, `change` tagged by `kind`; `null` otherwise |
| `kind: "alert"` | — | dark with `alerts` |
| — | `kind: "presence"`, `kind: "baseline"` | an address gone; a presence rule's first look |
| `id: "<rule>:<origin>/<producer>"` (`liveliness-gone`) | `"<rule>:<address>"`, `"<rule>:<address>/<iface>[/<member>]"`, `"<rule>:<token key>"`, `"<rule>:observer"` | |
| `id: "<rule>:<alert_ref>"` | — | dark with `alerts` |
| `rendering: schema \| structural \| key_only` | the same words; `key_only` on every rule today | the payload path returns with `alerts` |

### The state file

| v1 | zk2 | Notes |
|---|---|---|
| `"version": 1` | `"version": 2` | a version-1 file is **refused, never migrated** (exit 2, by `run` and `check-config`): its identities name v1 origins, producers and alert refs no zk2 rule produces. Move it aside to start fresh; the first run announces once what is still firing |
| — | `"namespace"` | the namespace the identities were judged in; a file written in another is refused |
| entries' `origin` | entries' `address` | |
| entries' `inhibited_by` | — | |

### What it publishes (v1's write side, until FL3)

| v1 | zk2 | Notes |
|---|---|---|
| keys `<base>/v1/<origin>/…` on the one session | the same keys, the namespace as the base, on the session in no namespace | FL3 replaces them with a `zenwatch.v1` contract served through `ServiceBuilder` |
| `health.inhibited` | always `0` | |
| `health.dropped_total`: the monitor's drops | the presence and token observers' drops | |
| `doctor.report`: v1's `DoctorReport` | zk2's `DoctorReport` | each check's verdict on the RFC 13 core, `scope` instead of the v1 counters |
