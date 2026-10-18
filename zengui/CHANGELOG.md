# zengui changelog

`zengui` ships as Forgejo release binaries, not on crates.io, like `zenctl`.
What it reads off a bus is the contract with its users, so a change to a
flag, a badge, a file or a surface is written down here, old beside new.

## Unreleased (`main`, zk2) — zengui on zk2 (#614)

zengui reads a zk2 deployment. v1's zengui, which the `v1` branch still
builds and releases as 0.14.x, read the keyspace-v2 grammar
(`<base>/v1/<origin>/<class>/<producer>/…`) through v1's registry slices,
introspection, roster and storages, and linked v1's `zenkey` and
`zenkey-fleet` from crates.io. Since #614 it runs on the in-tree zk2 engine
and links neither. It opens two sessions: the data pump watches wire
selectors in **no** namespace, and presence, descriptors and contracts are
read **in** the deployment's namespace. Each key resolves through the lens
to an identity rung, and its payload is checked against the contract its
provider's descriptor names. The tree starts from the contract skeleton,
and the Inspector reads a value by what the key turns out to be. The
Workbench reads nodes from presence and runs zk2's doctor, and Send
publishes foreign keys only and calls operations through their contracts.
A replay resolves through an offline lens. v1 keys are foreign keys now
(`not zk2`), with no v1 overlay. The surfaces that spoke v1's planes are
deleted, not stubbed: no zk2 profile serves blobs or media yet (#613). The
port landed in chunks FL5 to FL12 of #614 (epic #585). The Config tool,
deleted with them, is back on `config.v1` (#768, CD).

### Flags and the deployment

| Before | Now | Notes |
|---|---|---|
| `--base <BASE>` (env `ZENGUI_BASE`): flag or env, then the context, then empty | `--namespace <NAMESPACE>`, `--base` kept as an alias. Resolves `--namespace` > the namespace picked last (remembered in zengui's prefs) > env `ZENGUI_BASE` > the context's `base` > empty | Empty is the bus-root deployment, whose keys start at `zk2/`. `ZENGUI_BASE` is a rung below the remembered namespace now, not a clap `env` beside the flag |
| `--registry <DIR>` (repeatable): registry slices read from disk instead of the bus | refused by name before anything else is read, exit 2, naming `--contracts` | A registry slice describes nothing a zk2 bus carries. A context's stored `registry` is never read as contracts, and is carried through a save untouched |
| — | `--contracts <PATH>` (repeatable): an authoring file, a directory of them, or a `.history` root, loaded as `zenctl --contracts` loads them | They seed the contract store, so a revision held offline is never retrieved; every other revision a descriptor names is retrieved from its holders (spec §8.4). The Settings overlay remembers them in zengui's own prefs, never in the shared context. A stale remembered path is dropped for that launch; a typed one is checked strictly |
| one session, un-namespaced, for everything | two sessions, as zenctl and zenwatch hold them: the raw one (no namespace) for the pump, wire selectors and the admin space; the namespaced one for presence, descriptors, contracts, S4 reads, health, why and calls | A namespace switch reopens the namespaced session, the presence monitor and the lens, and forgets what they held |
| the base picker, fed by `discover_bases` | the namespace picker, fed by `namespace_listing` (instance tokens, `**/zk2/*/*/@zk/instance/*`) | |
| registry slices from introspection (RFC 08 §6), and the v1 roster from `alive` tokens and `@catalog` | the contract store (offline contracts plus bundles retrieved by fingerprint), and presence on a monitor of its own on the namespaced session, coalesced to four snapshots a second | Presence runs apart from the pump, so the pump's `Dropped(n)` cannot knock it to lost |
| status strip: `base:`, `registry: bus · n slices`, `facts: n cached`, `seeded n: … storage` | `namespace:`, `contracts: n offline · m retrieved · k unavailable`, `presence: n up · n down · n unobservable`, `skeleton: n revisions · n addresses · admin …`, `identities: n cached`, `seeded n: h history · s state · …`, `fetched <key>: <read>` | |

### Scopes

| Before | Now | Notes |
|---|---|---|
| `everything`: plain-chunk traffic, `@rpc`/`@media`/`@blob`/`@catalog` excluded | `everything` (`**`), `@stream`/`@state`/`@op`/`@zk` and the admin space excluded by key algebra (core §1.3) | Still the default. Every selector says which verbatim chunks it cannot see, in zk2's words |
| `deployment`: the three data classes of the base, plus `@catalog` | `deployment`: the namespace's ambient keys, `<namespace>/zk2/**` | |
| `telemetry` | `streams`: the ambient streams | `telemetry` is a serde and clap alias: a saved prefs file and a typed `--scope telemetry` read as `streams` |
| `state`, `events` | `state`, `events`: the deployment's ambient state, and its events | Built with zenkey-model's kind selectors |
| — | `explicit`: `@stream/**` and `@state/**`, named one by one | No ambient selector ever delivers a verbatim kind; spelling one is the only way in |
| `custom` | `custom` | |

### Badges

| Before | Now | Notes |
|---|---|---|
| registration, five states: `registered` ●, `unregistered` ○, `no slice` ✗, `—` (not asked) ·, `n/a` | the identity rung, fourteen states, one word each: `resolved`, `control key`, `not in namespace`, `not zk2`, `presence not read`, `no provider`, `no revision`, `ambiguous revision`, `contract not held`, `contract unavailable`, `contract unreadable`, `no resource`, `no member`, `decode not asked` | Five glyph classes: resolved ●, short ○ (no provider, no revision, ambiguous, no member), broken ✗ (contract unavailable or unreadable, no resource), not asked · (presence not read, contract not held, decode not asked), inapplicable — (not zk2, not in namespace, control key). The tree, the jump-to list and the Inspector use the same words |
| every v1 key badged by registration; a foreign key unbadged | a foreign key unbadged, except a key that claims a convention: a `not zk2` key whose first namespace-relative chunk is `zk2` or `v1`, and a `not in namespace` key that holds a `zk2` chunk | A v1 key reads `— not zk2`. Detail shows the rung for every key |
| payload verdict, three poles: valid ✓, invalid ✗, not validated · | conformance, four poles: valid ✓, invalid ✗, undecodable ⚠, not checked · (with the rung where resolution stopped) | Plus the verdict cache's own "not yet checked", which is never a verdict. Checked through `Lens::check` against the contract the descriptor names. A resolved rung gives way to the verdict beside it in a narrow row |
| budget: `over budget: observed n of d declared`, `exempt: rest-variable`, from v1's `cardinality` per origin | `exceeds budget: n of b` ⚠, `within budget: n of b` ✓, `budget unobservable (<why>)` ~, `budget not asked (<why>)` ·, from `zenkey_model::budget` (core §2.7) | Judged every 16 live ticks over a window built from each member's last sighting, when a covering watch began, drops and evictions, the longest gap between ticks, presence and the descriptor's bound. Most read unobservable and say why (`window too short`, `lossy`, `owner absent`, `a window does not count state`). None without a presence read, so none offline or in a replay |

### Tree

| Before | Now | Notes |
|---|---|---|
| roles `version`, `origin`, `class`, `producer`, `tier`, `subject` | roles `grammar` (`zk2`), `system`, `service`, `iface`, `kind`, `resource`, and `control` for `@zk` | Each chunk checked with the grammar's own types, relative to the namespace, whose chunks must lead the key. The location trail carries each segment's role |
| pivots `chunks`, `by origin`, `by producer`, `by class` | `chunks`, `by system`, `by address`, `by iface`, `by kind` | The Diagnose preset pivots by address, where it pivoted by origin. An interface group spans addresses, so it offers no one-click watch |
| the declared skeleton from v1's registry slices, origins symbolic (`{origin}`) | the contract skeleton from the lens: every resource of every address presence names, declared before a byte arrives, templates symbolic | Explicit `@stream` and `@state` leaves stay declared-only under `**`. A declared `@op` row offers no watch, and no watch, seed or fetch ever GETs under `@op`, because a GET there is a call. Rebuilt when the lens's inputs or the admin answer move, never on a tick |
| the late-joiner seed: an `@adv` history GET and a storage GET (consolidation `None`) | the history leg, and a state leg: one GET with target `All` and consolidation `Latest` (S4), merged with live samples by stamp | Seeds count `history`, `state`, `incomplete` and `superseded` |

### Nodes

| Before | Now | Notes |
|---|---|---|
| the roster: per producer per origin, from v1 `alive` tokens and `@catalog`; `alive`, or `suspect (token retracted)` | one card per **address** (`<system>/<service>`), from presence: `up`, `down` or `unobservable`, since when, its instances with theirs | Unobservable is never down: a loss makes every address unobservable. "Presence not read" is not an empty deployment |
| lazy `node_info` detail for the selected origin | the descriptor's facts (host, process, build, zid, profiles, interfaces), indexed once per catalog, and `health.v1` read once per selection, with a "read again" button | No ambient polling. A `hostid.v1` chip only when the descriptor's `profiles` lists it, never from a system's name |
| — | a "clocks synced" toggle, off by default | Without it a health status's age reads "age unobservable — …", as `zenctl health` does without `--clocks-synced` |
| the origin subject | the address subject: the Inspector's address sections, including why and consumers | The watched-freshness join stays, with `state/**` and `@state/**` named apart |

### Doctor

| Before | Now | Notes |
|---|---|---|
| v1's 23 checks: `slice-parse`, `slice-sync`, `introspect-coverage`, `admin-unreachable`, `router-version-skew`, `describe-totality`, `schema-drift`, `describe-missing`, `stale-state`, `unstamped-state`, `storage-coverage`, `payload-undecodable`, `payload-invalid`, `qos-observed-mismatch`, `unregistered-traffic`, `rate-over-declared`, `timestamp-stamped-elsewhere`, `cardinality-over-declared`, `field-vanished`, `field-stuck`, `field-new`, `kind-mismatch`, `budget-exceeded` | zk2's 19 (`zenkey_fleet::report::CheckId`): `split-brain`, `binding-unsatisfied`, `contract-drift`, `contract-unavailable`, `descriptor-invalid`, `token-missing`, `presence-over-budget`, `storage-on-state`, `archive-unaligned`, `state-stamp-foreign`, `shm-memlock-low`, `admin-unreachable`, `router-version-skew`, `health-failed`, `health-degraded`, `health-stale`, `health-inconsistent`, `hostid-duplicate`, `population-over-bound` | `run_doctor` on the namespaced and raw sessions, as `zenctl doctor`. Shown as findings by severity, then every check's verdict (finding, clean, unobservable or not asked, each with its reason), then what was not judged, then what a re-run fixed. An empty scope is no verdict |
| the LISTEN (S) input (#161) | removed | zk2's doctor runs no passive phase |
| "re-ask schemas" | "re-read presence and contracts": restarts the presence link and forgets the store's absences | |
| `deep` | `deep: GET every owner's state`, and a "clocks synced" toggle of its own | Its own toggle, as `zenctl doctor` and `zenctl health` each take `--clocks-synced` |
| a finding targets a v1 key or a node | a zk2 key, or a `<system>/<service>` address | |

### Admin and mesh

| Before | Now | Notes |
|---|---|---|
| the coverage table: declared state families against the storages that would seed them, "uncovered" a finding | "S4: storages on owners' state": per storage, `answers on owner state` (the finding), `not on owner state`, or `not judged` with why (`model::storage::on_owner_state`) | S4 inverts the old reading: a late joiner seeds from the owner, and last-known state is an `archive.v1`'s (§4.4). The late-joiner-seed text and the producer filter are gone |
| the topology with origin attachments | `topology_with_instances`, trust off: only verified routers attach instances | The topology line carries instance counts |
| the mesh: routers with origin satellites | `render_dot(&topology)`: instances drawn beside their router; unattached and unattributable ones counted | |

### Send

| Before | Now | Notes |
|---|---|---|
| a QoS picker over v1's five profiles (`sampled`, `refreshed`, `transition`, `alert`, `frame`) | wire axes: priority, congestion, reliability, express (`WireQos`) | Until touched: "zenoh's default — no contract types a foreign key". A tombstone rides the form's axes, not v1's `transition` |
| a call to an `@rpc` procedure, typed targets by origin, per-origin outcomes | a contract call on `@op`: pick an interface whose revision (offline or retrieved) declares the operation, an address (presence lists its providers), then `plan_call` → `encode_request` → `call_operation` on the namespaced session | Asked once, no retries, waiting the contract's `timeout_ms`. A forbidden fan-out (a `*` in the address, a parameter not given) is refused by the form before anything is sent (O2). A value, an envelope, a malformed reply and silence render apart (O5); silence is a non-verdict that lists what presence saw. The request form comes from `Revision::schema_view`: JSON Schema properties or protobuf fields, scaffolded to encode as its type. No attachment: `OperationCall` carries none |
| publish of any key, encoded through the engine's ladder (`prepare_publish`) | publish of a **foreign** key only: a zk2 data key a service owns is refused (P3, in `zenctl pub`'s words), as it is typed and again on send; a wildcard is refused too | `prepare_publish`, `PrepareSpec`, `PrepareMode` and `BodySource` are gone |
| retire | a tombstone through zk2's `check_retire`: any concrete key needs `--i-know`, a wildcard is refused | |
| the body as typed, or encoded by the ladder | the body as typed, or hex (two digits a byte, spaces ignored); bad hex is refused before anything is sent. The wire `Encoding` is what the form names, or none | The repeat stream and its matching badge stay; the send log stays bounded |

### Config (#768, CD)

The Config tool is back, on `config.v1` (`spec/profiles/config/v1.md`,
text 0.3). FL11a deleted v1's (#481), which spoke RFC 05 §5.1; this one is
built on the engine's reading, acts and §5 judgement
(`zenkey_fleet::{run_config, call_config}`, `model::config`,
`judge::config`), and holds no protocol logic of its own.

| Before | Now | Notes |
|---|---|---|
| — (deleted at FL11a) | `config` on the Workbench's strip; "configure" on a nodes card and in the Inspector's address view, only where a served descriptor lists `config.v1` (§2.1) | Picking the tool with an address selected reads it; while it is the Workbench's tool it follows the selected address |
| v1's read: one `@rpc` call, the producer's read-back | `run_config`: presence and the descriptor, one S4 GET of `config.v1/state/**`, each document judged by `freshness.v1`, §5's answers in their three states (`not yet`, `unobservable` and `not asked` kept apart) | A "clocks synced" toggle, as the nodes pane's: without it a reply's age is unobservable, and the tool says how to give it a ground |
| a target typed as origin, producer and resource | the address (`<system>/<service>`), its instance, revision and floor | |
| a pending change with the producer's deadline and a count from the send | the pending change's token, groups, deadline as the owner's instant (§2.8), and a countdown read against this host's clock, labelled so; this window's own margin (§2.8) and the reach reply's `apply_at` when it armed the change | The countdown ticks only while a change is pending. Near the deadline the tool listens across it once, so the undo's put and its `rolled-back` event are seen |
| confirm, cancel, extend, persist, by token | the same, through `call_config`: confirm offered only once the view shows the change applied (§2.9), persist on the last change | |
| a form per hot group | a form per writable group (§2.16), typed by the served schema (`type_values`) and refused in the form by `precheck` — the owner's own `validate_set` against the view read — before any task exists; the window asked, raised to the floor, shown before sending; a reach change behind the consent `consent` asks (§2.4); `dry run`; join the pending change | Each `set` carries the view's revision as `expected_revision`. An idempotency key is held only while its answer was silence, so a resend is a retry the owner answers once (§2.14) |
| a reply classified by hand | each answer by its union (§3): `applied` or `pending`, `view` or `status`, the envelope with its `ConfigError`, malformed, undecodable, or silence — a non-verdict, never a failure; the call metadata claimed (`$USER`, a ULID) | |
| a sensitive value write-only in the row | write-only everywhere (§2.15): a masked field never prefilled, held as a `Masked` whose `Debug` is `<redacted>`, sent as a value read, so it is `<redacted>` in the request shown and scrubbed from what comes back; a redacted edit in an event | Nothing of the tool is persisted in prefs |
| — | every act refused during a replay, in every window, as publish and call are | Reads stay allowed |
| the change events: none | the change events a reading heard (a window, a replay), kept for the service shown, bounded at 16, newest first | |

### Value (the Inspector)

| Before | Now | Notes |
|---|---|---|
| `fetch_value`: a plain GET answered by storage, then the `@adv` cache, then the live window (`storage`, `@adv cache`, `live window`) | the read follows the key's identity (`value::plan`), decided before any session is touched | |
| storage | a zk2 **state** key: S4's owner GET (`get_state`), `via owner GET (S4)`, its reply's bytes in hex beside the decode (#765). No answer reads "silent", never "no value" | Stamps are attributed through `Lens::provenance` (O7). Bytes that do not decode as their type get a structural reading |
| — | **last-known**: `last_known_state`, behind one button per `archive.v1` provider presence shows, landing beside the owner's read as "last-known from <archive> — not current (S5, S6)" | Never shown as current |
| `@adv cache` | a **stream** with `history`: the owner's `@adv` cache (§2.5). A stream without history is not read, and the pane says why | |
| live window | an **event**: replayed on a button press only, bounded by its retention (§2.6). A **foreign** or control key: a raw `fleet_get`, labelled `raw GET`; a tombstone in its answer reads as one, with a `DELETE (tombstone)` chip where `0 B` would be (#765) | Under `@op`, on a symbolic path, or a zk2 key the lens stopped short of: nothing is read, and the pane says why |
| a fetch on select during a replay, through the live bus | no fetch during a replay: each slot shows the capture's latest row at the playhead, checked through the replay's offline lens | Leaving the replay re-reads every slot live |
| declared QoS: the registry's v1 profile (`qos_verdict`) | the resource's declared axes (`declared_qos`) against the newest sample's (`observed_qos`), compared by `qos_mismatch`, each differing axis named with both sides | Reliability is the link's: shown, not compared. A v1 or v2 row reads "QoS not recorded" and draws no verdict |
| the facts ladder: key shape, registration, origin kind, producer | the identity section: rung, and where resolution stopped; ADDRESS, IFACE, KIND, RESOURCE, TYPE, REVISION and the template values; the type section from `type_view` | The series says it has no unit: zk2 contracts carry none |
| why: v1's `run_why` rungs | zk2's `run_why` on a key (`WhyTarget::Key`) or an address (`WhyTarget::Service`): the namespace and key rungs judged offline, a stream key listened to for 3 s; `not_asked`, never "no" | Never under `@op` |
| fields: `run_field` through v1 slices and schemas | `run_field(raw, &Lens, &FieldSpec)`, its window and bounds stated | |
| consumers: an admin-space join | R3 edges from descriptors (`Catalog::graph()`), for a key's interface or an address, with no bus | "R3: from descriptors"; never a matching status (D13) |

### Files

| Before | Now | Notes |
|---|---|---|
| record: `.zrec` version 2 | `.zrec` version 3: the header's `excluded`, each row's `qos_axes` | Replay reads versions 1 to 3. A version-1 or -2 row named a v1 profile, which zk2 does not read: it reads "QoS not recorded", and the banner counts those rows |
| the `.zsnap` compare: v1 rows (registration, verdict, holder) | `.zsnap` version 2 (`ZsnapReader`): each row's identity, conformance, stamper and holder | A version-1 file is refused, naming the `v1` branch, which reads it |
| echo export: NDJSON rows with `origin` and `subject` | zk2 `SampleRow`s with `identity`, resolved through the live lens, or the replay's while one runs | `origin` and `subject` are gone. The row leaves out `type`, `typed`, a verdict and `qos_axes`, which zenctl's rows carry, because the echo ring keeps none of them |

### Replay

| Before | Now | Notes |
|---|---|---|
| a replay merged against the live skeleton and checked through the live verdicts | resolved through an offline lens (`ReplayLens`): no catalog, the offline contracts, and the capture's namespace (the header's `base` for a file, the live namespace for the retained window) | Its skeleton is the tree's declared half while it runs. The live verdict cache is cleared on entry and exit, replayed ticks are checked offline, a live batch landing during a replay is dropped, and identities come from a replay-scoped cache. Revisions retrieved live are not used: they describe the live world |
| — | the banner's second line: the header's `excluded` (or "not stated" for v1 and v2 files), "QoS not recorded" counted by row, and "resolved offline: n revisions held, no presence read" | |
| recording refused during a replay; publish still possible | publish, retire, call and recording refused during a replay in every window, their controls disabled; why, fields and consumers refused too (O4) | Entering a replay stops an armed repeat publication |

### Removed

| Before | Now | Notes |
|---|---|---|
| the Config tool (#481): RFC 05 §5.1 configuration calls | deleted at FL11a, then back on `config.v1` (#768, CD; see Config above) | No stub while the profile was missing: a control that does nothing is a promise the window cannot keep. A prefs file naming the Config pane still loads, with the Workbench opening on Send |
| the Blob browser (#68): RFC 07 §2's `@blob` | deleted | No `blob.v1` yet (#613) |
| the Media viewer (#69): RFC 07 §1's `@media` | deleted | No `media.v1` yet (#613). An explicit `@stream` is the zk2 counterpart, reachable with the `explicit` scope |
| `examples/spray_v1.rs` (the v1 demo traffic), and the `spray-v1`, `gui-demo-v1` and `gui-demo-no-registry` recipes | `examples/spray.rs`, a zk2 deployment in namespace `spray`; `gui-demo` (`--namespace spray --contracts examples/zk2/.history`), `gui-demo-no-contracts`, `gui-demo-bounded` | The spray serves its own bundles, so `gui-demo-no-contracts` shows retrieval from the bus |

### Benches

| Before | Now | Notes |
|---|---|---|
| `tree/pivot_50k_producer` | `tree/pivot_50k_address` | `docs/bench-baseline.md`'s figures were measured under the old name and code |
| `roster/refresh_40p_8w` | `nodes/refresh_40a_8w` | The same join, over forty addresses on presence where it was forty producers on the roster, with eight watches |
