# zenwatch changelog

`zenwatch` ships as Forgejo release binaries, never on crates.io. A
notifier's config is written once and edited rarely, so this file has to be
complete enough that a v1 config can be moved in one sitting: every row
names where its question went, or that nothing on `main` asks it yet. No
aliases and no shims, except where the two spellings were always one fact
(`base` and `namespace`).

## Unreleased (`main`, zk2) — `instance-gone`, one reading with zenctl (#747)

The address form's reading — `instance-gone`, and `liveliness-gone
…/@zk/instance/*` — moved into the engine (`zenkey_fleet::AddressWatch`),
and `zenctl watchdog`'s `instance-gone` reads the same one: what an
address held and gone means, the first look, and what a pattern can call
gone are decided once. Nothing a rule notifies changed, but one sentence:
the observer back under a complete read says "every address under … is
observable", without "again", in the words the watchdog's rule uses.

## Unreleased (`main`, zk2) — the write side on zk2 (#614, FL3)

The daemon is a zk2 service of its own: `health.v1`, through the runtime,
and its own contract, `zenwatch.v1` (`contracts/zenwatch.v1.toml`, its
append-only `.history`), on the session in the deployment's namespace,
brought up through `zenkey::ServiceBuilder` in spec §8.2's order. zenwatch
no longer links v1's libraries: `zenkey =0.11.1`, `zenkey-build =0.11.1`
and `zenkey-fleet =0.18.0` are gone from it, and with them
`registry/zenwatch.kdl`.

### What it publishes

| v1 | zk2 | Notes |
|---|---|---|
| origin `h-<12hex>` from the `zenwatch-host-id-v1` salt (`AppProfile`) | the address `@hostid.v1/zenwatch`: the system minted from the machine id with zk2's salt (`hostid.v1`), the service `zenwatch` | the same machine id gives a different system (hostid.v1 Appendix B maps one to the other). A host with no id refuses to start, exit 2 (§2.6); `publish.address` and `publish.ephemeral` below are the ways out |
| keys `<ns>/v1/<origin>/state/zenwatch/…` on the session in no namespace | `zk2/<system>/zenwatch/<iface>/…` on the session in the namespace | the namespace is the session's, as every zk2 owner's |
| `…/@rpc/zenwatch/introspect` (the registry slice) | the descriptor and the bundles (spec §3.3, §8.4) | `zenctl service show`, `zenctl schema show zenwatch.v1` |
| `…/@rpc/zenwatch/describe` (the schema set) | the bundle's `schemas/zenwatch.json` | Rust-first: what the daemon's structs derive, committed and held to them |
| the `…/state/zenwatch/alive` token | the instance token, then the interface tokens (`zenwatch.v1`, `health.v1`) | `instance-gone */zenwatch` watches a zenwatch |
| `…/state/zenwatch/health` every 30 s, `status: ok \| degraded` | `health.v1`: `state/status` (`OK`, `DEGRADED`, `FAILED`), re-put every 28.5 s; `state/checks/{check}` | the level is the worst of the checks `sinks`, `presence`, `tokens`, `doctor`, `state-file`, `firing` (see the README); `degraded` "a delivery failed since the last document" became `sinks` at each sink's last delivery, which stays failed until one goes out; all of them failing is `FAILED` |
| `health.host_id` | the address, and the descriptor's `meta.host` | the system is in the key (health.v1 Appendix A) |
| `health.inhibited` | — | inhibition was removed in FL2 |
| `health.last_persist_error` | the `state-file` check's detail | |
| `health.doctor`, `doctor_last`, `doctor_next` | the `doctor` check, and `zenwatch.v1`'s `state/doctor` (`ran_at`, `next_at`) | |
| `health.notifications_sent`, `deliveries_failed`, `dropped_total`, `state_entries`, `state_evicted`, `firing_refused` | `zenwatch.v1` `state/counters`: `discipline.sent`, `deliveries_failed`, `dropped`, `state_entries`, `discipline.evicted`, `firing_refused` | a count is not a level: counters are not health.v1 checks. New beside them: `delivered`, `forgotten`, `doctor_runs`, and every discipline counter (`deduped`, `cancelled`, `baseline`, `grouped`, `repeats`) |
| `health.started_at`, `rules`, `sinks`, `firing`, `unobservable` | `state/counters`, same names | `freshness.ttl_s` 60; put on change, at most once per tick |
| `…/state/zenwatch/firing/{rule_id}` (`ttl_s` 300), tombstoned when quiet | `zenwatch.v1` `state/firing/{rule_id}` (`freshness.ttl_s` 300), deleted when quiet | same document. The bound of 256 is the runtime's now (core §2.7, `Error::Budget`): a refusal is counted once per rule refused, not once per tick |
| `…/state/zenwatch/doctor` (`ttl_s` 0, retained) | `zenwatch.v1` `state/doctor` (no horizon; optional, gated `config:doctor`) | same document; unavailable (cause `config`) in the descriptor without a `doctor` block. Not deleted at a clean stop: an archive keeps it as last-known |
| at shutdown: `firing/*` and `health` tombstoned, then the token | the last tick, the observers stopped and the sinks drained, then `firing/*` deleted and the tokens withdrawn | the health status is left, as health.v1 §2.3 asks; the daemon never pages about its own clean stop |

### Configuration

| v1 | zk2 | Notes |
|---|---|---|
| — (always published under the salted origin) | `publish: {}` (the default) | `@hostid.v1/zenwatch` |
| — | `publish: { address: "<system>/<service>" }` | a literal system, nothing minted; or `@hostid.v1/<service>` to rename the service |
| — | `publish: { ephemeral: true }` | allow hostid.v1's ephemeral system on a host with no id (§2.6): one run's, another at the next start. Refused beside a literal address |
| — | `publish: false` | no service at all |
| `check-config` | `check-config` mints the address too | `run` and `check-config` pass one gate: a host with no id is refused by both, before any session opens |

## Unreleased (`main`, zk2) — the read side on zk2 (#614, FL2)

zenwatch's rules, presence, contracts and doctor read a zk2 deployment
through the in-tree engine (`zenkey-fleet` 0.20, by path), on two
sessions: one in the deployment's namespace, one in none (decided
2026-10-08). What the daemon publishes about itself stayed v1's until
FL3 (above).

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

### What it publishes (FL2's interim; FL3 replaced it, above)

| v1 | zk2 | Notes |
|---|---|---|
| keys `<base>/v1/<origin>/…` on the one session | the same keys, the namespace as the base, on the session in no namespace | until FL3 |
| `health.dropped_total`: the monitor's drops | the presence and token observers' drops | `counters.dropped` since FL3 |
| `doctor.report`: v1's `DoctorReport` | zk2's `DoctorReport` | each check's verdict on the RFC 13 core, `scope` instead of the v1 counters |
